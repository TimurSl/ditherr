use std::fmt;

/// Errors produced by the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// A parameter is out of range or inconsistent.
    InvalidParam(String),
    /// The image has no pixels (or the pipeline reduced it to none).
    EmptyImage,
    /// A preset palette id is not known.
    UnknownPreset(String),
    /// A look file was written by a newer version of Ditherr.
    UnsupportedVersion(u32),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidParam(msg) => write!(f, "invalid parameter: {msg}"),
            Error::EmptyImage => write!(f, "image has no pixels"),
            Error::UnknownPreset(id) => write!(f, "unknown palette preset '{id}'"),
            Error::UnsupportedVersion(v) => {
                write!(f, "look file version {v} is newer than this build supports")
            }
        }
    }
}

impl std::error::Error for Error {}
