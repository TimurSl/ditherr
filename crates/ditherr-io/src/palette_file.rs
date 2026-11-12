//! Palette files (spec §5): GIMP `.gpl`, Lospec `.hex`, JASC `.pal` and Adobe
//! `.act`. A palette holds 1 to 256 sRGB colours, the same limit as
//! [`PaletteSpec::Custom`](ditherr_core::PaletteSpec).

use std::path::Path;

use crate::error::{Error, Result};

/// The most colours a palette can hold.
pub const MAX_COLORS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaletteFile {
    /// Only `.gpl` carries a name; other formats read back an empty one.
    pub name: String,
    pub colors: Vec<[u8; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteFormat {
    Gpl,
    Hex,
    Pal,
    Act,
}

impl PaletteFormat {
    pub const ALL: [PaletteFormat; 4] = [
        PaletteFormat::Gpl,
        PaletteFormat::Hex,
        PaletteFormat::Pal,
        PaletteFormat::Act,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            PaletteFormat::Gpl => "gpl",
            PaletteFormat::Hex => "hex",
            PaletteFormat::Pal => "pal",
            PaletteFormat::Act => "act",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.extension() == ext)
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Palette(msg.into()))
}

fn check_count(n: usize) -> Result<()> {
    match n {
        0 => err("the file has no colours"),
        n if n > MAX_COLORS => err(format!(
            "{n} colours; palettes are limited to {MAX_COLORS} colours"
        )),
        _ => Ok(()),
    }
}

pub fn parse_palette(bytes: &[u8], format: PaletteFormat) -> Result<PaletteFile> {
    match format {
        PaletteFormat::Act => parse_act(bytes),
        _ => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::Palette("not a text file".into()))?
                .trim_start_matches('\u{feff}');
            match format {
                PaletteFormat::Gpl => parse_gpl(text),
                PaletteFormat::Hex => parse_hex(text),
                _ => parse_pal(text),
            }
        }
    }
}

pub fn encode_palette(palette: &PaletteFile, format: PaletteFormat) -> Result<Vec<u8>> {
    check_count(palette.colors.len())?;
    Ok(match format {
        PaletteFormat::Gpl => encode_gpl(palette).into_bytes(),
        PaletteFormat::Hex => encode_hex(palette).into_bytes(),
        PaletteFormat::Pal => encode_pal(palette).into_bytes(),
        PaletteFormat::Act => encode_act(palette),
    })
}

pub fn read_palette(path: impl AsRef<Path>) -> Result<PaletteFile> {
    let path = path.as_ref();
    let format = PaletteFormat::from_path(path).ok_or_else(|| {
        Error::Palette(format!(
            "{}: unknown palette format; use .gpl, .hex, .pal or .act",
            path.display()
        ))
    })?;
    let bytes = std::fs::read(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    parse_palette(&bytes, format).map_err(|e| match e {
        Error::Palette(m) => Error::Palette(format!("{}: {m}", path.display())),
        other => other,
    })
}

pub fn write_palette(path: impl AsRef<Path>, palette: &PaletteFile) -> Result<()> {
    let path = path.as_ref();
    let format = PaletteFormat::from_path(path).ok_or_else(|| {
        Error::Palette(format!(
            "{}: unknown palette format; use .gpl, .hex, .pal or .act",
            path.display()
        ))
    })?;
    let bytes = encode_palette(palette, format)?;
    std::fs::write(path, bytes).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })
}

// ---- GIMP .gpl -----------------------------------------------------------

fn parse_gpl(text: &str) -> Result<PaletteFile> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("GIMP Palette") {
        return err("missing the \"GIMP Palette\" header");
    }
    let mut out = PaletteFile::default();
    for (i, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix("Name:") {
            out.name = name.trim().to_string();
            continue;
        }
        if line.starts_with("Columns:") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let mut channel = || -> Option<u8> { parts.next()?.parse().ok() };
        match (channel(), channel(), channel()) {
            (Some(r), Some(g), Some(b)) => out.colors.push([r, g, b]),
            _ => return err(format!("line {}: expected \"R G B [name]\"", i + 2)),
        }
    }
    check_count(out.colors.len())?;
    Ok(out)
}

fn encode_gpl(p: &PaletteFile) -> String {
    let name = if p.name.trim().is_empty() {
        "Ditherr palette"
    } else {
        p.name.trim()
    };
    // The name is one line in the file.
    let name = name.replace(['\r', '\n'], " ");
    let mut s = format!("GIMP Palette\nName: {name}\nColumns: 0\n#\n");
    for c in &p.colors {
        s.push_str(&format!(
            "{:3} {:3} {:3}\t{:02x}{:02x}{:02x}\n",
            c[0], c[1], c[2], c[0], c[1], c[2]
        ));
    }
    s
}

// ---- Lospec .hex ---------------------------------------------------------

fn parse_hex(text: &str) -> Result<PaletteFile> {
    let mut out = PaletteFile::default();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with(';') {
            continue;
        }
        let digits = line.strip_prefix('#').unwrap_or(line);
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return err(format!("line {}: expected a 6-digit hex colour", i + 1));
        }
        // Six hex digits always parse.
        let v = u32::from_str_radix(digits, 16).unwrap_or(0);
        out.colors.push([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    check_count(out.colors.len())?;
    Ok(out)
}

fn encode_hex(p: &PaletteFile) -> String {
    p.colors
        .iter()
        .map(|c| format!("{:02x}{:02x}{:02x}\n", c[0], c[1], c[2]))
        .collect()
}

// ---- JASC .pal -----------------------------------------------------------

fn parse_pal(text: &str) -> Result<PaletteFile> {
    let mut lines = text.lines().map(str::trim);
    if lines.next() != Some("JASC-PAL") {
        return err("missing the \"JASC-PAL\" header");
    }
    if lines.next() != Some("0100") {
        return err("unsupported JASC-PAL version");
    }
    let Some(Ok(count)) = lines.next().map(str::parse::<usize>) else {
        return err("missing colour count");
    };
    check_count(count)?;
    let mut out = PaletteFile::default();
    for (i, line) in lines.filter(|l| !l.is_empty()).take(count).enumerate() {
        let mut parts = line.split_whitespace();
        let mut channel = || -> Option<u8> { parts.next()?.parse().ok() };
        match (channel(), channel(), channel()) {
            (Some(r), Some(g), Some(b)) => out.colors.push([r, g, b]),
            _ => return err(format!("colour {}: expected \"R G B\"", i + 1)),
        }
    }
    if out.colors.len() != count {
        return err(format!(
            "the header says {count} colours but the file has {}",
            out.colors.len()
        ));
    }
    Ok(out)
}

fn encode_pal(p: &PaletteFile) -> String {
    let mut s = format!("JASC-PAL\n0100\n{}\n", p.colors.len());
    for c in &p.colors {
        s.push_str(&format!("{} {} {}\n", c[0], c[1], c[2]));
    }
    s
}

// ---- Adobe .act ----------------------------------------------------------

fn parse_act(bytes: &[u8]) -> Result<PaletteFile> {
    let count = match bytes.len() {
        768 => 256,
        772 => {
            let n = usize::from(u16::from_be_bytes([bytes[768], bytes[769]]));
            // 0 and 0xFFFF both mean "no count": use the whole table.
            if n == 0 || n > MAX_COLORS {
                256
            } else {
                n
            }
        }
        n => {
            return err(format!(
                "{n} bytes; an Adobe colour table is 768 or 772 bytes"
            ))
        }
    };
    let colors = bytes[..count * 3]
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    Ok(PaletteFile {
        name: String::new(),
        colors,
    })
}

fn encode_act(p: &PaletteFile) -> Vec<u8> {
    let mut out = vec![0u8; 768];
    for (i, c) in p.colors.iter().enumerate() {
        out[i * 3..i * 3 + 3].copy_from_slice(c);
    }
    if p.colors.len() < MAX_COLORS {
        // Colour count, then the transparency index (none).
        out.extend_from_slice(&(p.colors.len() as u16).to_be_bytes());
        out.extend_from_slice(&0xFFFFu16.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(n: usize) -> PaletteFile {
        PaletteFile {
            name: "Test".into(),
            colors: (0..n)
                .map(|i| {
                    [
                        (i * 37 % 256) as u8,
                        (i * 91 % 256) as u8,
                        (i * 5 % 256) as u8,
                    ]
                })
                .collect(),
        }
    }

    #[test]
    fn every_format_round_trips_colours() {
        for n in [1, 2, 7, 16, 255, 256] {
            let p = sample(n);
            for f in PaletteFormat::ALL {
                let bytes = encode_palette(&p, f).unwrap();
                let back = parse_palette(&bytes, f).unwrap();
                assert_eq!(back.colors, p.colors, "{f:?} with {n} colours");
            }
        }
    }

    #[test]
    fn gpl_keeps_its_name() {
        let p = sample(3);
        let bytes = encode_palette(&p, PaletteFormat::Gpl).unwrap();
        assert_eq!(
            parse_palette(&bytes, PaletteFormat::Gpl).unwrap().name,
            "Test"
        );
    }

    #[test]
    fn parses_hand_written_gimp_palette() {
        let text = "GIMP Palette\nName: Sunset\nColumns: 4\n# a comment\n255   0   0\tRed\n  0 128 255 Sky blue\n";
        let p = parse_palette(text.as_bytes(), PaletteFormat::Gpl).unwrap();
        assert_eq!(p.name, "Sunset");
        assert_eq!(p.colors, vec![[255, 0, 0], [0, 128, 255]]);
    }

    #[test]
    fn hex_accepts_hashes_comments_and_blank_lines() {
        let text = "// my colours\n\n#FF0000\n00ff00\r\n  0000FF  \n";
        let p = parse_palette(text.as_bytes(), PaletteFormat::Hex).unwrap();
        assert_eq!(p.colors, vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
    }

    #[test]
    fn act_uses_the_count_field() {
        let bytes = encode_palette(&sample(5), PaletteFormat::Act).unwrap();
        assert_eq!(bytes.len(), 772);
        assert_eq!(
            parse_palette(&bytes, PaletteFormat::Act)
                .unwrap()
                .colors
                .len(),
            5
        );
        // Exactly 256 colours need no trailer.
        let full = encode_palette(&sample(256), PaletteFormat::Act).unwrap();
        assert_eq!(full.len(), 768);
        // A bare 768-byte table is all 256 entries.
        assert_eq!(
            parse_palette(&[0; 768], PaletteFormat::Act)
                .unwrap()
                .colors
                .len(),
            256
        );
        // 0xFFFF means "unspecified".
        let mut open = vec![0u8; 772];
        open[768] = 0xFF;
        open[769] = 0xFF;
        assert_eq!(
            parse_palette(&open, PaletteFormat::Act)
                .unwrap()
                .colors
                .len(),
            256
        );
    }

    #[test]
    fn rejects_bad_files_with_a_reason() {
        let bad = |text: &str, f| parse_palette(text.as_bytes(), f).unwrap_err().to_string();
        assert!(bad("nope", PaletteFormat::Gpl).contains("GIMP Palette"));
        assert!(bad("GIMP Palette\n1 2\n", PaletteFormat::Gpl).contains("line 2"));
        assert!(bad("GIMP Palette\n", PaletteFormat::Gpl).contains("no colours"));
        assert!(bad("12345\n", PaletteFormat::Hex).contains("line 1"));
        assert!(bad("zzzzzz\n", PaletteFormat::Hex).contains("hex"));
        assert!(bad("JASC-PAL\n0100\n2\n1 2 3\n", PaletteFormat::Pal).contains("2 colours"));
        assert!(bad("PAL\n", PaletteFormat::Pal).contains("JASC-PAL"));
        assert!(parse_palette(&[0; 10], PaletteFormat::Act).is_err());
        assert!(parse_palette(&[0xff, 0xfe], PaletteFormat::Gpl).is_err());
    }

    #[test]
    fn size_limits() {
        let too_many = "000000\n".repeat(257);
        assert!(parse_palette(too_many.as_bytes(), PaletteFormat::Hex)
            .unwrap_err()
            .to_string()
            .contains("257"));
        assert!(encode_palette(&sample(257), PaletteFormat::Hex).is_err());
        assert!(encode_palette(&sample(0), PaletteFormat::Gpl).is_err());
    }

    #[test]
    fn format_from_path() {
        assert_eq!(
            PaletteFormat::from_path(Path::new("a/B.GPL")),
            Some(PaletteFormat::Gpl)
        );
        assert_eq!(PaletteFormat::from_path(Path::new("x.png")), None);
    }

    #[test]
    fn files_on_disk() {
        let dir = std::env::temp_dir().join(format!("ditherr-pal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = sample(9);
        for f in PaletteFormat::ALL {
            let path = dir.join(format!("p.{}", f.extension()));
            write_palette(&path, &p).unwrap();
            assert_eq!(read_palette(&path).unwrap().colors, p.colors);
        }
        assert!(read_palette(dir.join("missing.gpl")).is_err());
        assert!(write_palette(dir.join("p.png"), &p).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
