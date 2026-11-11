# Universal Dithering Software: Functional Specification

Nov 11, 2026 · @Tymur

## 1. Overview

The product is one shared dithering engine with several workspaces on top, so a look created for a still image also applies to a GIF, a video, or a 3D model's textures.

It serves casual users who want a result in seconds and professionals who need every parameter: pixel artists, game developers, print designers, motion designers, and hardware makers.

| Layer | What it contains |
| --- | --- |
| Core engine | Pre-processing, dithering algorithms, parameters, palette and quantization, masking |
| Workspaces | Quick Stylize, Studio, Animation, 3D Texture, Batch |
| Additional modes | Sprite and pixel art, Print, Hardware, Live input |
| Shared systems | Presets, palette library, preview, project files |
| Output | Indexed images, animation, texture sets, vector, shaders, device data |

Design rules that apply everywhere:

- **One engine:** every workspace calls the same processing pipeline, so results match across modes.
- **Non-destructive:** the source is never altered; all settings stay editable.
- **Real-time:** every setting change shows in the preview without a render step.
- **Portable looks:** a preset is a file that works in any workspace.

## 2. Core engine: input and pre-processing

Pre-processing runs before quantization, because the image fed to the dither affects the result as much as the algorithm does.

- **Image formats:** PNG, JPEG, TIFF, WebP, BMP, PSD, and RAW, with 16-bit and alpha support.
- **Other inputs:** GIF, APNG, video, image sequences, 3D models with textures, clipboard, and drag and drop.
- **Resize:** target size in pixels or percent, with nearest, bilinear, bicubic, and Lanczos resampling.
- **Tone:** exposure, contrast, gamma, levels, curves, and black and white points.
- **Color:** saturation, hue shift, channel mixer, and custom weights for grayscale conversion.
- **Detail:** sharpen, unsharp mask, blur, and noise reduction.
- **Geometry:** crop, rotate, and flip.
- **Color management:** ICC profile reading and conversion to a working space.

## 3. Core engine: dithering algorithms

The engine ships six algorithm families and lets users define their own kernels and matrices.

| Family | Algorithms | Typical use |
| --- | --- | --- |
| Error diffusion | Floyd-Steinberg, Jarvis-Judice-Ninke, Stucki, Burkes, Sierra (3-row, 2-row, Lite), Atkinson, Stevenson-Arce | Photos, smooth gradients, classic 1-bit looks |
| Ordered | Bayer 2x2 to 16x16, clustered dot, blue noise (void-and-cluster), user threshold maps | Animation, textures, retro game looks |
| Halftone | Amplitude-modulated screens with round, ellipse, line, diamond, and cross dots | Print, newspaper and comic looks |
| Stochastic | White noise, blue noise, random threshold | Film-grain looks, low-artifact output |
| Pattern and curve | Riemersma (Hilbert curve), Yliluoma, Knoll, dot diffusion | Limited palettes, artifact-free gradients |
| Basic | Plain threshold, posterize without dither | Hard-edged graphics, comparison baseline |

Custom algorithm tools:

- **Kernel editor:** a grid for defining error-diffusion weights and divisor.
- **Matrix editor:** a grid for defining ordered-dither threshold maps, with import from an image.
- **Algorithm library:** custom kernels and matrices saved, named, and shared as files.

## 4. Core engine: algorithm parameters

Every algorithm exposes the parameters below where they apply.

- **Strength:** the share of quantization error that is diffused, from 0% to above 100%.
- **Scan order:** serpentine or left-to-right, with a choice of scan direction.
- **Threshold bias:** shifts the overall balance lighter or darker.
- **Threshold jitter:** adds noise to break up worm and stripe artifacts.
- **Error clamping:** limits how far error can accumulate, which reduces color bleeding.
- **Linear-light processing:** a gamma-correct toggle; without it results come out visibly too dark or too light.
- **Pattern transform:** scale, rotation, and offset of the matrix or halftone screen.
- **Halftone screen:** dot shape, angle, and frequency, with separate angles per CMYK channel.
- **Pixel scale:** dither at a lower resolution, then upscale with nearest-neighbor for a chunky look.
- **Per-channel settings:** a different algorithm or strength for each color channel.
- **Edge-aware dithering:** less dither on edges, more in gradients.
- **Random seed:** a fixed seed so stochastic results are repeatable.

## 5. Core engine: palette and quantization

The palette decides which colors the dither may use, and the distance metric decides which one it picks for each pixel.

- **Bit depth:** 1-bit, N-level grayscale, or levels per channel such as RGB 3-3-2 and RGB565.
- **Automatic palettes:** median cut, octree, k-means, and NeuQuant, with a color count from 2 to 256.
- **Preset palettes:** Game Boy, CGA, EGA, C64, NES, PICO-8, ZX Spectrum, Macintosh 1-bit, web-safe, and e-ink panels.
- **Palette editor:** add, remove, lock, and reorder colors, and pick colors from the image.
- **Palette files:** import and export of .gpl, .ase, .act, .pal, and .hex, plus extraction from any image.
- **Color distance metric:** RGB, weighted RGB, CIELAB (Delta E 76 and 2000), or Oklab.
- **Tone mapping to inks:** duotone and tritone, with a custom color assigned to each tonal range.
- **Shared palettes:** one palette computed across many images, frames, or textures.

## 6. Core engine: masking and selective control

Masks let different parts of one image receive different treatment.

- **Region masks:** painted, gradient, or imported masks that limit where dithering applies.
- **Per-region algorithms:** a different algorithm, palette, or strength inside each mask.
- **Tonal ranges:** separate settings for shadows, midtones, and highlights.
- **Alpha handling:** dithered transparency, a hard threshold, or alpha preserved as-is.
- **Blending:** the dithered result mixed with the original using opacity and blend modes.

## 7. Workspace: Quick Stylize

Quick Stylize gives a finished image in a few clicks and hides everything except five controls.

- **One-click looks:** a gallery of presets shown as live thumbnails of the user's own image.
- **Starter looks:** Game Boy, newspaper halftone, 1-bit Macintosh, CGA, e-ink, and risograph.
- **Five sliders:** pixel size, color count, dither strength, contrast, and palette.
- **Randomize:** generates variations to browse and keep.
- **Open in Studio:** hands the current look to the full editor with every setting exposed.
- **Quick export:** PNG or GIF at 1x, 2x, or 4x in one step.

## 8. Workspace: Studio

Studio is the professional still-image editor and exposes the whole core engine.

- **Effect stack:** pre-processing, quantization, and dither steps as reorderable, toggleable layers.
- **Full parameter panels:** every setting from sections 2 to 6.
- **Mask tools:** brush, gradient, and selection tools for region masks.
- **Comparison views:** split, side-by-side, and A/B toggle against the original.
- **Variations:** several saved versions of one image kept side by side.
- **Info panel:** histogram, count of colors used, and estimated file size.
- **History:** unlimited undo and redo with a visible step list.

## 9. Workspace: Animation

Animation applies the engine to GIFs and video, where the main problem is keeping the dither stable from frame to frame.

- **Timeline:** scrubbing, trimming, frame rate change, and frame skipping.
- **Pattern behavior:** locked to the frame (stable), animated per frame (grain), or locked to moving content through motion estimation.
- **Flicker suppression:** temporal smoothing for error diffusion, which shimmers between frames otherwise.
- **Palette strategy:** one global palette, per-frame palettes, or per-scene palettes with cut detection.
- **Keyframed settings:** strength, pixel size, palette, and other parameters animated over time.
- **GIF optimization:** frame differencing, transparency reuse, loop control, and a target file size.
- **Playback preview:** real-time playback at the output frame rate.
- **Formats:** GIF, APNG, animated WebP, MP4, WebM, and image sequences, in and out.

## 10. Workspace: 3D Texture

3D Texture stylizes a model's textures and solves the problems flat images do not have: seams, uneven texel density, and mipmaps.

- **Model import:** OBJ, FBX, and glTF with their materials and textures.
- **3D viewport:** live preview on the model with orbit, a lighting toggle, and nearest-neighbor filtering.
- **Seam-aware dithering:** the pattern continues across UV island edges instead of breaking at them.
- **Tileable output:** the dither wraps at the texture edges for repeating materials.
- **Texel density control:** dither dots stay the same size across the model even when UV islands differ in scale.
- **Map-aware processing:** color maps (albedo, emissive) are dithered, roughness and metallic can be quantized, and normal maps are left untouched by default.
- **Shared palette:** one palette across all textures of a model or a whole asset set.
- **Mipmap handling:** mips generated per level from the source, or disabled, so the dither does not blur at a distance.
- **Texture-space and screen-space preview:** compares dither baked into the texture with a post-process look.
- **Shader export:** the same look as a screen-space shader for Unity, Unreal, Godot, or GLSL.
- **Engine-ready export:** texture sets with naming conventions, indexed or truecolor output, and atlas support.

## 11. Workspace: Batch and automation

Batch runs any preset over many files without opening each one.

- **Queue:** folders of images, GIFs, videos, or texture sets processed with one preset.
- **Naming templates:** output names and folders built from source name, preset, and size.
- **Watch folders:** new files processed automatically, for use in an asset pipeline.
- **Command-line interface:** the same engine and presets with no window.
- **Scripting and plugin API:** custom algorithms, importers, and exporters.
- **Job report:** a log of processed files, failures, and output sizes.

## 12. Additional modes

Four specialist modes reuse the engine for narrower jobs.

| Mode | Functions |
| --- | --- |
| Sprite and pixel art | Sprite sheet slicing, per-tile processing, outline preservation, palette-swap variants |
| Print | Halftone separations, spot colors, DPI and LPI settings, vector output for screen printing and risograph |
| Hardware | E-ink, thermal printer, and LED matrix targets with device previews and byte-array export |
| Live input | Webcam or screen capture through the engine in real time, for streaming and live visuals |

## 13. Shared systems

These systems are available in every workspace and are what make the product one tool instead of five.

- **Universal presets:** a look saved as a file that applies to an image, a GIF, a video, or a texture set.
- **Preset library:** built-in and user presets with thumbnails, tags, search, import, and export.
- **Palette library:** one collection of palettes shared by all workspaces.
- **GPU preview:** real-time results in every workspace, including video playback and the 3D viewport.
- **Pixel-accurate zoom:** 100%, 200%, and higher with no smoothing, plus an optional pixel grid.
- **Project files:** sources, effect stacks, masks, and export settings saved together.
- **Interface:** dockable panels, keyboard shortcuts, light and dark themes, and localization.

## 14. Output and export

Export must keep every pixel exactly as previewed, so indexed formats and integer scaling are the defaults.

| Output | Formats and options |
| --- | --- |
| Still image | PNG at true bit depth (1, 2, 4, or 8-bit indexed), GIF, BMP, TIFF, WebP lossless |
| Animation | GIF, APNG, animated WebP, MP4, WebM, image sequence |
| Texture set | Per-map files with naming conventions, atlases, optional mip chains |
| Vector | Halftones as SVG or PDF |
| Print | CMYK separations, one file per channel or spot color, embedded ICC profile |
| Shader | Unity, Unreal, Godot, and GLSL screen-space versions of a look |
| Device data | Raw byte arrays, C headers, and device-specific formats |
| Palette | .gpl, .ase, .act, .pal, .hex |

Export options common to all raster outputs:

- **Integer upscaling:** 2x, 3x, 4x, or custom, with nearest-neighbor only.
- **Metadata:** kept, stripped, or replaced.
- **Export presets:** saved combinations of format, scale, and naming.

## 15. Priorities

The first release needs the core engine, Quick Stylize, Studio, and basic GIF support; the 3D Texture workspace is the main differentiator and comes second.

| Tier | Scope |
| --- | --- |
| Must have | Core engine with linear-light processing and perceptual color distance; error diffusion, Bayer, and blue noise; palette editor and presets; Quick Stylize; Studio; GIF in and out with a global palette; true indexed PNG export; real-time preview |
| Should have | Full Animation workspace with temporal stability; 3D Texture workspace with seam-aware dithering and viewport; batch queue and CLI; custom kernel and matrix editors; masks |
| Later | Shader export; Print and Hardware modes; Sprite mode; Live input; scripting and plugin API; motion-locked patterns |

Open questions:

- [x] Target platforms: desktop only, or web and mobile as well? desktop
- [x] Licensing model: one-time purchase, subscription, or open source? open-source
- [x] Is the 3D Texture workspace in the first release or the second? first
