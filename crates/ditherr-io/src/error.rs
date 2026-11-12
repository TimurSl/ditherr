use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// Decoding an image failed.
    Decode(String),
    /// Encoding or an unsupported output request.
    Encode(String),
    /// A look file could not be parsed or serialised.
    Look(String),
    /// A palette file could not be parsed or written.
    Palette(String),
    /// A project file could not be parsed or written.
    Project(String),
    Core(ditherr_core::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Decode(m) => write!(f, "cannot read image: {m}"),
            Error::Encode(m) => write!(f, "cannot write image: {m}"),
            Error::Look(m) => write!(f, "look file: {m}"),
            Error::Palette(m) => write!(f, "palette file: {m}"),
            Error::Project(m) => write!(f, "project file: {m}"),
            Error::Core(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Core(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<ditherr_core::Error> for Error {
    fn from(e: ditherr_core::Error) -> Self {
        Error::Core(e)
    }
}
