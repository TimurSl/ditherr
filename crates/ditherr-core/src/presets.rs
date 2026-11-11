//! Built-in preset palettes (spec §5).

/// How a preset is represented.
#[derive(Debug, Clone, Copy)]
pub enum PresetKind {
    /// An explicit colour list as `0xRRGGBB`.
    List(&'static [u32]),
    /// A full grid with this many evenly spaced levels per channel. Handled
    /// per channel by the quantizer.
    Grid { r: u16, g: u16, b: u16 },
}

#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: PresetKind,
}

const GAME_BOY: &[u32] = &[0x0f380f, 0x306230, 0x8bac0f, 0x9bbc0f];

/// CGA mode 4, palette 1, high intensity.
const CGA: &[u32] = &[0x000000, 0x55ffff, 0xff55ff, 0xffffff];

/// The 16 RGBI colours shared by CGA text mode and the default EGA palette.
const CGA16: &[u32] = &[
    0x000000, 0x0000aa, 0x00aa00, 0x00aaaa, 0xaa0000, 0xaa00aa, 0xaa5500, 0xaaaaaa, 0x555555,
    0x5555ff, 0x55ff55, 0x55ffff, 0xff5555, 0xff55ff, 0xffff55, 0xffffff,
];

/// Commodore 64, Pepto's measurements.
const C64: &[u32] = &[
    0x000000, 0xffffff, 0x68372b, 0x70a4b2, 0x6f3d86, 0x588d43, 0x352879, 0xb8c76f, 0x6f4f25,
    0x433900, 0x9a6759, 0x444444, 0x6c6c6c, 0x9ad284, 0x6c5eb5, 0x959595,
];

/// NES. The console's output is not standardised, so this is a widely used
/// approximation of the 2C02 palette with duplicate blacks removed.
const NES: &[u32] = &[
    0x626262, 0x001fb2, 0x2404c8, 0x5200b2, 0x730076, 0x800024, 0x730b00, 0x522800, 0x244400,
    0x005700, 0x005c00, 0x005324, 0x003c76, 0x000000, 0xababab, 0x0d57ff, 0x4b30ff, 0x8a13ff,
    0xbc08d6, 0xd21269, 0xc72e00, 0x9d5400, 0x607b00, 0x209800, 0x00a300, 0x009942, 0x007db4,
    0xffffff, 0x53aeff, 0x9085ff, 0xd365ff, 0xff57ff, 0xff5dcf, 0xff7757, 0xfa9e00, 0xbdc700,
    0x7ae700, 0x43f611, 0x26ef7e, 0x2cd5f6, 0x4d4d4d, 0xb6e1ff, 0xced1ff, 0xe9c3ff, 0xffbcff,
    0xffbdf4, 0xffc6c3, 0xffd59a, 0xe9e681, 0xcef481, 0xb6fb9a, 0xa9fac3, 0xa9f0f4, 0xb8b8b8,
];

const PICO8: &[u32] = &[
    0x000000, 0x1d2b53, 0x7e2553, 0x008751, 0xab5236, 0x5f574f, 0xc2c3c7, 0xfff1e8, 0xff004d,
    0xffa300, 0xffec27, 0x00e436, 0x29adff, 0x83769c, 0xff77a8, 0xffccaa,
];

/// ZX Spectrum: normal (0xD7) and bright (0xFF) colours; bright black is black.
const ZX_SPECTRUM: &[u32] = &[
    0x000000, 0x0000d7, 0xd70000, 0xd700d7, 0x00d700, 0x00d7d7, 0xd7d700, 0xd7d7d7, 0x0000ff,
    0xff0000, 0xff00ff, 0x00ff00, 0x00ffff, 0xffff00, 0xffffff,
];

const MAC_1BIT: &[u32] = &[0x000000, 0xffffff];

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "gameboy",
        name: "Game Boy",
        kind: PresetKind::List(GAME_BOY),
    },
    Preset {
        id: "cga",
        name: "CGA (mode 4, palette 1)",
        kind: PresetKind::List(CGA),
    },
    Preset {
        id: "cga16",
        name: "CGA / EGA 16 colours",
        kind: PresetKind::List(CGA16),
    },
    Preset {
        id: "ega",
        name: "EGA (64 colours)",
        kind: PresetKind::Grid { r: 4, g: 4, b: 4 },
    },
    Preset {
        id: "c64",
        name: "Commodore 64",
        kind: PresetKind::List(C64),
    },
    Preset {
        id: "nes",
        name: "NES (approximate)",
        kind: PresetKind::List(NES),
    },
    Preset {
        id: "pico8",
        name: "PICO-8",
        kind: PresetKind::List(PICO8),
    },
    Preset {
        id: "zx-spectrum",
        name: "ZX Spectrum",
        kind: PresetKind::List(ZX_SPECTRUM),
    },
    Preset {
        id: "mac-1bit",
        name: "Macintosh 1-bit",
        kind: PresetKind::List(MAC_1BIT),
    },
    Preset {
        id: "web-safe",
        name: "Web-safe (216 colours)",
        kind: PresetKind::Grid { r: 6, g: 6, b: 6 },
    },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

pub fn unpack(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_unique_and_lists_have_no_duplicates() {
        let mut ids = HashSet::new();
        for p in PRESETS {
            assert!(ids.insert(p.id), "duplicate id {}", p.id);
            if let PresetKind::List(l) = p.kind {
                let set: HashSet<_> = l.iter().collect();
                assert_eq!(set.len(), l.len(), "{} has duplicate colours", p.id);
                assert!(l.len() <= 256);
            }
        }
    }

    #[test]
    fn known_sizes() {
        let len = |id| match find(id).unwrap().kind {
            PresetKind::List(l) => l.len(),
            PresetKind::Grid { r, g, b } => usize::from(r) * usize::from(g) * usize::from(b),
        };
        assert_eq!(len("gameboy"), 4);
        assert_eq!(len("cga"), 4);
        assert_eq!(len("cga16"), 16);
        assert_eq!(len("ega"), 64);
        assert_eq!(len("c64"), 16);
        assert_eq!(len("pico8"), 16);
        assert_eq!(len("zx-spectrum"), 15);
        assert_eq!(len("mac-1bit"), 2);
        assert_eq!(len("web-safe"), 216);
        assert!(len("nes") >= 50);
    }
}
