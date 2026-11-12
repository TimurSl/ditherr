//! Decoding and encoding for Ditherr: images, indexed PNG, GIF, BMP, and look,
//! palette and project files.

mod error;
mod export;
mod import;
mod look_file;
mod palette_file;
mod project_file;

pub use error::{Error, Result};
pub use export::{encode, encode_indexed_png, save, Format};
pub use import::{decode_image, read_image};
pub use look_file::{look_to_string, parse_look, read_look, write_look, LOOK_EXTENSION};
pub use palette_file::{
    encode_palette, parse_palette, read_palette, write_palette, PaletteFile, PaletteFormat,
    MAX_COLORS,
};
pub use project_file::{
    parse_project, project_to_string, read_project, write_project, Edit, ExportSettings, Project,
    Variation, MAX_EXPORT_FACTOR, PROJECT_EXTENSION, PROJECT_VERSION,
};
