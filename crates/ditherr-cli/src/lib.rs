//! The `ditherr` command line. A thin wrapper: everything pixel-related happens
//! in `ditherr-core`, file handling in `ditherr-io`.

use std::fmt;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use ditherr_core::extract::extract;
use ditherr_core::{presets, AutoMethod, Look, Rendered};

#[derive(Debug, Parser)]
#[command(
    name = "ditherr",
    version,
    about = "Dither images with the Ditherr engine"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Apply a look to an image.
    Apply(ApplyArgs),
    /// Work with palettes.
    Palette {
        #[command(subcommand)]
        command: PaletteCommand,
    },
}

#[derive(Debug, Args)]
pub struct ApplyArgs {
    /// Input image (PNG, JPEG, WebP, BMP, TIFF, GIF).
    pub input: PathBuf,
    /// Look file (.ditherlook). Without one, a default 1-bit Floyd-Steinberg look is used.
    #[arg(short, long)]
    pub look: Option<PathBuf>,
    /// Output file; the extension picks the format (.png, .gif, .bmp, .tif, .webp).
    #[arg(short, long)]
    pub output: PathBuf,
    /// Integer upscale applied to the result. Defaults to the look's pixel scale, which
    /// brings the image back to its original size (rounded up to a multiple of the scale).
    #[arg(long, value_name = "N")]
    pub upscale: Option<u32>,
    /// Override the look's random seed.
    #[arg(long)]
    pub seed: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum PaletteCommand {
    /// List the built-in preset palettes.
    List,
    /// Extract a palette from an image and print it as hex, one colour per line.
    Extract {
        input: PathBuf,
        /// Number of colours (2 to 256).
        #[arg(short = 'n', long, default_value_t = 16)]
        colors: u16,
        #[arg(short, long, value_enum, default_value_t = Method::MedianCut)]
        method: Method,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Method {
    MedianCut,
    Octree,
    Kmeans,
}

impl From<Method> for AutoMethod {
    fn from(m: Method) -> Self {
        match m {
            Method::MedianCut => AutoMethod::MedianCut,
            Method::Octree => AutoMethod::Octree,
            Method::Kmeans => AutoMethod::KMeans,
        }
    }
}

#[derive(Debug)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E: std::error::Error> From<E> for CliError {
    fn from(e: E) -> Self {
        CliError(e.to_string())
    }
}

/// Runs a parsed command, writing human-readable output to `out`.
pub fn run(cli: Cli, out: &mut impl std::io::Write) -> Result<(), CliError> {
    match cli.command {
        Command::Apply(args) => apply(&args, out),
        Command::Palette { command } => palette(command, out),
    }
}

fn apply(args: &ApplyArgs, out: &mut impl std::io::Write) -> Result<(), CliError> {
    let mut look = match &args.look {
        Some(path) => ditherr_io::read_look(path)?,
        None => Look::default(),
    };
    if let Some(seed) = args.seed {
        look.params.seed = seed;
    }
    let factor = args.upscale.unwrap_or(look.pixel_scale);
    if factor == 0 {
        return Err(CliError("--upscale must be at least 1".into()));
    }

    let src = ditherr_io::read_image(&args.input)?;
    let rendered = ditherr_core::render(&src, &look)?.upscale(factor);
    ditherr_io::save(&args.output, &rendered)?;

    let detail = match &rendered {
        Rendered::Indexed(i) => {
            format!("{} colours, {}-bit indexed", i.palette.len(), i.bit_depth())
        }
        Rendered::Truecolor(_) => "truecolour".to_string(),
    };
    writeln!(
        out,
        "wrote {} ({}x{}, {detail})",
        args.output.display(),
        rendered.width(),
        rendered.height()
    )?;
    Ok(())
}

fn palette(cmd: PaletteCommand, out: &mut impl std::io::Write) -> Result<(), CliError> {
    match cmd {
        PaletteCommand::List => {
            for p in presets::PRESETS {
                let n = match p.kind {
                    presets::PresetKind::List(l) => l.len(),
                    presets::PresetKind::Grid { r, g, b } => {
                        usize::from(r) * usize::from(g) * usize::from(b)
                    }
                };
                writeln!(out, "{:<12} {:>3} colours  {}", p.id, n, p.name)?;
            }
        }
        PaletteCommand::Extract {
            input,
            colors,
            method,
        } => {
            if !(2..=256).contains(&colors) {
                return Err(CliError("--colors must be between 2 and 256".into()));
            }
            let img = ditherr_io::read_image(&input)?;
            let pixels: Vec<[u8; 3]> = img
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 127)
                .map(|p| [p[0], p[1], p[2]])
                .collect();
            for c in extract(method.into(), &pixels, usize::from(colors)) {
                writeln!(out, "{:02x}{:02x}{:02x}", c[0], c[1], c[2])?;
            }
        }
    }
    Ok(())
}
