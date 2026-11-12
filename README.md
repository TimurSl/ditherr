# Ditherr

An open-source desktop dithering tool: one shared engine, several workspaces on top
(Quick Stylize, Studio, Animation, 3D Texture, Batch). See
[`docs/Universal Dithering Software Functional Specification.md`](docs/Universal%20Dithering%20Software%20Functional%20Specification.md).

**Status:** milestone M3: the engine, the CLI, the Quick Stylize window and the Studio workspace
work. Animation, 3D Texture and batch are still to come.

## Quick start

```sh
nix develop            # or install stable Rust
cargo run --release -p ditherr-cli -- apply photo.jpg --look looks/gameboy.ditherlook -o out.png
cargo run --release -p ditherr-cli -- palette list
cargo run --release -p ditherr-cli -- palette extract photo.jpg -n 8 --method kmeans
cargo run --release -p ditherr-app -- photo.jpg    # Quick Stylize window
cargo run --release -p ditherr-app -- shot.ditherproj   # open a Studio project
```

## Quick Stylize

`ditherr-app [image]` opens a window with a gallery of live thumbnails (Game Boy, Newspaper,
Macintosh 1-bit, CGA, E-ink, Risograph), five controls (pixel size, colours, dither strength,
contrast, palette), **Randomize** for variations, and PNG/GIF export at 1x, 2x or 4x. Drop an image
on the window or use **Open…**; hold Space to compare with the original. The preview renders on a
background thread through `ditherr-core`; images over 2048 px are shrunk for preview only, and
export always renders the full-size original. At 1x an export matches the source size.

## Studio

The **Studio** tab (or **Open in Studio** in Quick Stylize, which hands the current look over) shows
every engine setting. Layout, left to right:

- **Effect stack and history.** Pre-process steps (exposure, contrast, gamma, levels, saturation,
  grayscale, resize, crop, rotate, flips) are layers you can reorder, switch off without losing their
  settings, add and delete. They run top to bottom, followed by the fixed stages pixel scale, palette
  and dither. Below the stack: saved **variations** (click to apply, right-click to delete) and the
  **history** list, with unlimited undo and redo (Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y). One slider drag is
  one step.
- **Canvas.** Result, A/B against the original (checkbox, or hold Space), **split** with a draggable
  divider, or **side by side**. Pixel-accurate zoom from 100% to 1600% with Ctrl+scroll or the +/−
  buttons, drag to pan, double-click to fit. The pointer shows the pixel position and colour. Zoom
  applies to the preview image, which is shrunk for sources over 2048 px.
- **Parameters.** Every setting of the selected layer or stage: algorithm and kernel, strength,
  serpentine and scan direction, threshold bias, jitter, error clamp, linear light, seed, pixel
  scale, colour distance metric, background. The **palette editor** edits a custom colour list
  (colour picker, hex, add, delete, reorder, sort), imports and exports palette files, extracts
  colours from the image or from any image file, and can pick colours by clicking the image.
- **Info panel.** A histogram of the tones going into the dither, how much of the result each
  colour takes, the number of colours used, and an estimated PNG size at the export scale.

**Files.** A **project** (`.ditherproj`) stores the source image path (relative to the project file
when it lies below it), the layers including disabled ones, the look, saved variations and export
settings. A **look** (`.ditherlook`) works in every workspace and the CLI, but cannot hold disabled
layers, so those are left out when saving one. **Palette files** are GIMP `.gpl`, Lospec `.hex`,
JASC `.pal` and Adobe `.act`. Studio exports PNG, GIF, BMP, TIFF and lossless WebP at 1x to 16x.

Not in Studio yet: mask tools and per-region settings (they need masking in the engine first),
custom kernel and matrix editors, locking palette colours, and `.ase` palettes.

`apply` writes indexed PNG at true bit depth (1, 2, 4 or 8) when the palette allows it; GIF, BMP,
TIFF and lossless WebP output are chosen by file extension. The `looks/` directory holds example
`.ditherlook` files (regenerate with `cargo run -p ditherr-io --example gen_looks`).

Palette files work from the command line too:

```sh
ditherr palette extract photo.jpg -n 8 -m kmeans -o photo.gpl   # or .hex, .pal, .act
ditherr palette convert photo.gpl photo.hex
ditherr apply photo.jpg --look looks/gameboy.ditherlook --palette photo.gpl -o out.png
```

## What the engine does

- **Algorithms:** nine error-diffusion kernels, Bayer 2x2 to 16x16, blue noise, white noise,
  threshold, posterize.
- **Palettes:** 1-bit, N-level gray, RGB 3-3-2 / RGB565 / any per-channel grid, ten presets
  (Game Boy, CGA, EGA, C64, NES, PICO-8, ZX Spectrum, Mac 1-bit, web-safe), median cut, octree,
  k-means; distance metrics RGB, weighted RGB, ΔE76, ΔE2000, Oklab.
- **Pre-processing:** resize (nearest, bilinear, bicubic, Lanczos), exposure, contrast, gamma,
  levels, saturation, weighted grayscale, crop, rotate, flip.
- **Parameters:** strength, serpentine and scan direction, bias, jitter, error clamping,
  linear-light toggle, pixel scale, seed.

Everything is deterministic: the same look gives the same bytes on every platform. Non-linear
maths goes through the pure-Rust `libm` crate (a clippy lint enforces it), and
`crates/ditherr-core/tests/golden.txt` holds hashes of every algorithm and preset that CI checks
on Linux, Windows and macOS. If engine output changes on purpose, regenerate them with
`UPDATE_GOLDEN=1 cargo test -p ditherr-core --test golden`.

## Layout

| Crate | Role |
| --- | --- |
| `ditherr-core` | Pure, deterministic engine. No I/O, no GPU. |
| `ditherr-io` | Image, GIF, palette, look and project file I/O. |
| `ditherr-3d` | Model import, UV/seam analysis, texture sets. |
| `ditherr-cli` | `ditherr` command-line tool. |
| `ditherr-app` | egui application (Quick Stylize and Studio). |

The GUI and CLI contain no pixel logic; everything goes through `ditherr-core`.

## Development

With Nix: `nix develop` (or `direnv allow`) gives you the Rust toolchain and the
windowing/graphics libraries. Otherwise install stable Rust.

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
