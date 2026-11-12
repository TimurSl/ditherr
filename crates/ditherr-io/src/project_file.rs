//! Project files (spec §13): the source image, the editable effect stack and
//! look, saved variations and export settings, stored together as RON.
//!
//! The source image is referenced, not embedded. It is stored relative to the
//! project file when it lives below it, so a folder can be moved or shared.

use std::path::{Path, PathBuf};

use ditherr_core::stack::{enabled_steps, layers_from_steps};
use ditherr_core::{Layer, Look};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::export::Format;

pub const PROJECT_VERSION: u32 = 1;
pub const PROJECT_EXTENSION: &str = "ditherproj";

/// Largest integer upscale a project may ask for.
pub const MAX_EXPORT_FACTOR: u32 = 16;

/// Everything about how the image is processed, and the unit of undo.
///
/// The pre-process stack lives in `layers` so steps can be disabled; the
/// `look` field's own `preprocess` list is kept empty. [`Edit::look`] joins
/// them into the look the engine renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edit {
    pub layers: Vec<Layer>,
    pub look: Look,
}

impl Default for Edit {
    fn default() -> Self {
        Self::from_look(Look::default())
    }
}

impl Edit {
    /// Moves the look's pre-process steps into enabled layers.
    pub fn from_look(mut look: Look) -> Self {
        let layers = layers_from_steps(&std::mem::take(&mut look.preprocess));
        Self { layers, look }
    }

    /// The look to render: the enabled layers become the pre-process stack.
    pub fn look(&self) -> Look {
        Look {
            preprocess: enabled_steps(&self.layers),
            ..self.look.clone()
        }
    }

    /// Restores the invariant that `look.preprocess` is empty (a hand-edited
    /// file may have put steps there; they run before the layers).
    fn normalize(&mut self) {
        if !self.look.preprocess.is_empty() {
            let mut moved = layers_from_steps(&std::mem::take(&mut self.look.preprocess));
            moved.append(&mut self.layers);
            self.layers = moved;
        }
    }

    pub fn validate(&self) -> Result<()> {
        for layer in &self.layers {
            layer.step.validate()?;
        }
        self.look().validate()?;
        Ok(())
    }
}

/// A named, saved edit shown next to the working copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variation {
    pub name: String,
    pub edit: Edit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    /// File extension of the output format, for example `png`.
    pub format: String,
    /// Integer upscale applied on top of the look's pixel scale.
    pub factor: u32,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: "png".into(),
            factor: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Project {
    /// Absolute, or relative to the working directory, as the app opened it.
    pub source: Option<PathBuf>,
    pub edit: Edit,
    pub variations: Vec<Variation>,
    pub export: ExportSettings,
}

impl Project {
    pub fn validate(&self) -> Result<()> {
        self.edit.validate()?;
        for v in &self.variations {
            v.edit.validate()?;
        }
        if Format::from_extension(&self.export.format).is_none() {
            return Err(Error::Project(format!(
                "unknown export format \"{}\"",
                self.export.format
            )));
        }
        if !(1..=MAX_EXPORT_FACTOR).contains(&self.export.factor) {
            return Err(Error::Project(format!(
                "export scale must be between 1 and {MAX_EXPORT_FACTOR}"
            )));
        }
        Ok(())
    }
}

/// What is actually written: the same data, with the source as a portable string.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct ProjectDoc {
    version: u32,
    source: Option<String>,
    edit: Edit,
    variations: Vec<Variation>,
    export: ExportSettings,
}

impl Default for ProjectDoc {
    fn default() -> Self {
        Self {
            version: PROJECT_VERSION,
            source: None,
            edit: Edit::default(),
            variations: Vec::new(),
            export: ExportSettings::default(),
        }
    }
}

/// `source` as stored: relative to `dir` with `/` separators when it lies below it.
fn stored_source(source: &Path, dir: &Path) -> String {
    let path = source.strip_prefix(dir).unwrap_or(source);
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn resolve_source(stored: &str, dir: &Path) -> PathBuf {
    let p = Path::new(stored);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}

/// Serialises a project as RON. `dir` is the folder the file will live in.
pub fn project_to_string(project: &Project, dir: &Path) -> Result<String> {
    project.validate()?;
    let doc = ProjectDoc {
        version: PROJECT_VERSION,
        source: project.source.as_deref().map(|s| stored_source(s, dir)),
        edit: project.edit.clone(),
        variations: project.variations.clone(),
        export: project.export.clone(),
    };
    let body = ron::ser::to_string_pretty(&doc, ron::ser::PrettyConfig::default())
        .map_err(|e| Error::Project(e.to_string()))?;
    Ok(format!(
        "// Ditherr project, format version {PROJECT_VERSION}\n{body}\n"
    ))
}

/// Parses and validates a project. `dir` is the folder the file was read from;
/// a relative source path is resolved against it.
pub fn parse_project(text: &str, dir: &Path) -> Result<Project> {
    let mut doc: ProjectDoc = ron::from_str(text).map_err(|e| Error::Project(e.to_string()))?;
    if doc.version > PROJECT_VERSION {
        return Err(Error::Project(format!(
            "format version {} is newer than this build understands ({PROJECT_VERSION})",
            doc.version
        )));
    }
    doc.edit.normalize();
    for v in &mut doc.variations {
        v.edit.normalize();
    }
    let project = Project {
        source: doc.source.as_deref().map(|s| resolve_source(s, dir)),
        edit: doc.edit,
        variations: doc.variations,
        export: doc.export,
    };
    project.validate()?;
    Ok(project)
}

fn parent_dir(path: &Path) -> &Path {
    path.parent().unwrap_or_else(|| Path::new(""))
}

pub fn read_project(path: impl AsRef<Path>) -> Result<Project> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    parse_project(&text, parent_dir(path)).map_err(|e| match e {
        Error::Project(m) => Error::Project(format!("{}: {m}", path.display())),
        other => other,
    })
}

pub fn write_project(path: impl AsRef<Path>, project: &Project) -> Result<()> {
    let path = path.as_ref();
    let text = project_to_string(project, parent_dir(path))?;
    std::fs::write(path, text).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::{Algorithm, ErrorKernel, PaletteSpec, Step};

    fn busy_project() -> Project {
        let mut edit = Edit::from_look(Look {
            name: "busy".into(),
            preprocess: vec![Step::Exposure(0.5), Step::Contrast(0.2), Step::FlipVertical],
            pixel_scale: 3,
            palette: PaletteSpec::Custom(vec![[1, 2, 3], [200, 100, 50]]),
            algorithm: Algorithm::ErrorDiffusion(ErrorKernel::Atkinson),
            ..Look::default()
        });
        edit.layers[1].enabled = false;
        Project {
            source: Some(PathBuf::from("/tmp/photos/cat.png")),
            variations: vec![
                Variation {
                    name: "Soft".into(),
                    edit: edit.clone(),
                },
                Variation {
                    name: "Plain".into(),
                    edit: Edit::default(),
                },
            ],
            edit,
            export: ExportSettings {
                format: "gif".into(),
                factor: 4,
            },
        }
    }

    #[test]
    fn edit_joins_enabled_layers_into_the_look() {
        let look = Look {
            preprocess: vec![Step::Exposure(1.0), Step::Gamma(1.2)],
            ..Look::default()
        };
        let mut edit = Edit::from_look(look.clone());
        assert!(edit.look.preprocess.is_empty());
        assert_eq!(edit.look(), look);

        edit.layers[0].enabled = false;
        assert_eq!(edit.look().preprocess, vec![Step::Gamma(1.2)]);
    }

    #[test]
    fn round_trips_including_disabled_layers() {
        let p = busy_project();
        let text = project_to_string(&p, Path::new("/tmp/proj")).unwrap();
        let back = parse_project(&text, Path::new("/tmp/proj")).unwrap();
        assert_eq!(back, p);
        assert!(!back.edit.layers[1].enabled);
    }

    #[test]
    fn source_is_relative_below_the_project_folder() {
        let mut p = busy_project();
        p.source = Some(PathBuf::from("/work/shots/a/cat.png"));
        let text = project_to_string(&p, Path::new("/work/shots")).unwrap();
        assert!(text.contains("\"a/cat.png\""), "{text}");
        let back = parse_project(&text, Path::new("/elsewhere")).unwrap();
        assert_eq!(back.source, Some(PathBuf::from("/elsewhere/a/cat.png")));

        // Outside the folder it stays absolute.
        p.source = Some(PathBuf::from("/other/cat.png"));
        let text = project_to_string(&p, Path::new("/work/shots")).unwrap();
        let back = parse_project(&text, Path::new("/anywhere")).unwrap();
        assert_eq!(back.source, Some(PathBuf::from("/other/cat.png")));
    }

    #[test]
    fn missing_fields_take_defaults() {
        let p = parse_project("()", Path::new(".")).unwrap();
        assert_eq!(p, Project::default());
    }

    #[test]
    fn steps_in_the_look_are_moved_into_layers() {
        let p = parse_project(
            "(edit: (look: (preprocess: [Contrast(0.3)]), layers: [(step: FlipVertical)]))",
            Path::new("."),
        )
        .unwrap();
        assert!(p.edit.look.preprocess.is_empty());
        assert_eq!(
            p.edit.look().preprocess,
            vec![Step::Contrast(0.3), Step::FlipVertical]
        );
        assert!(p.edit.layers.iter().all(|l| l.enabled));
    }

    #[test]
    fn rejects_bad_projects() {
        let dir = Path::new(".");
        let bad = |t: &str| parse_project(t, dir).unwrap_err().to_string();
        assert!(bad("(version: 9)").contains("newer"));
        assert!(bad("(export: (format: \"xyz\"))").contains("xyz"));
        assert!(bad("(export: (factor: 0))").contains("between 1 and"));
        assert!(bad("(export: (factor: 99))").contains("between 1 and"));
        assert!(bad("(edit: (look: (pixel_scale: 0)))").contains("pixel scale"));
        assert!(bad("(edit: (layers: [(step: Contrast(-2.0))]))").contains("contrast"));
        assert!(bad("not ron").starts_with("project file:"));
    }

    #[test]
    fn writes_and_reads_files() {
        let dir = std::env::temp_dir().join(format!("ditherr-proj-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("img")).unwrap();
        let mut p = busy_project();
        p.source = Some(dir.join("img/cat.png"));
        let path = dir.join(format!("a.{PROJECT_EXTENSION}"));
        write_project(&path, &p).unwrap();
        assert_eq!(read_project(&path).unwrap(), p);
        assert!(read_project(dir.join("missing.ditherproj")).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
