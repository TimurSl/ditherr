# Ditherr

An open-source desktop dithering tool: one shared engine, several workspaces on top
(Quick Stylize, Studio, Animation, 3D Texture, Batch). See
[`docs/Universal Dithering Software Functional Specification.md`](docs/Universal%20Dithering%20Software%20Functional%20Specification.md).

**Status:** milestone M1: the engine and CLI work; there is no GUI yet.

## Quick start

```sh
nix develop            # or install stable Rust
cargo run --release -p ditherr-cli -- apply photo.jpg --look looks/gameboy.ditherlook -o out.png
cargo run --release -p ditherr-cli -- palette list
cargo run --release -p ditherr-cli -- palette extract photo.jpg -n 8 --method kmeans
```

`apply` writes indexed PNG at true bit depth (1, 2, 4 or 8) when the palette allows it; GIF, BMP,
TIFF and lossless WebP output are chosen by file extension. The `looks/` directory holds example
`.ditherlook` files (regenerate with `cargo run -p ditherr-io --example gen_looks`).

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
| `ditherr-app` | egui application. |

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
