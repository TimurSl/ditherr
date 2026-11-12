use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_ditherr");

fn looks_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../looks")
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ditherr-cli-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_input(path: &Path, w: u32, h: u32) {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            (x * 255 / (w - 1)) as u8,
            (y * 255 / (h - 1)) as u8,
            ((x + y) * 4 % 256) as u8,
            255,
        ])
    });
    img.save(path).unwrap();
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn applies_the_documented_example() {
    let dir = Scratch::new("example");
    let input = dir.path("input.png");
    let output = dir.path("out.png");
    write_input(&input, 66, 48);
    let look = looks_dir().join("gameboy.ditherlook");
    let out = run(&["apply", s(&input), "--look", s(&look), "-o", s(&output)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("66x48") && stdout.contains("4 colours, 2-bit indexed"),
        "{stdout}"
    );
    let img = image::open(&output).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (66, 48));
}

#[test]
fn every_shipped_look_applies_and_matches_the_library() {
    let dir = Scratch::new("looks");
    let input = dir.path("input.png");
    write_input(&input, 60, 40);
    let mut count = 0;
    for entry in std::fs::read_dir(looks_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("ditherlook") {
            continue;
        }
        count += 1;
        let output = dir.path("out.png");
        let out = run(&["apply", s(&input), "--look", s(&path), "-o", s(&output)]);
        assert!(
            out.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );

        // One-engine check: the CLI must produce exactly what the library does.
        let look = ditherr_io::read_look(&path).unwrap();
        let src = ditherr_io::read_image(&input).unwrap();
        let expected = ditherr_core::render(&src, &look)
            .unwrap()
            .upscale(look.pixel_scale);
        let got = ditherr_io::read_image(&output).unwrap();
        assert!(
            got == expected.to_rgba8(),
            "{} differs from the library result",
            path.display()
        );
    }
    assert!(count >= 5, "expected the shipped looks, found {count}");
}

#[test]
fn defaults_to_one_bit_and_honours_upscale_and_seed() {
    let dir = Scratch::new("defaults");
    let input = dir.path("input.png");
    write_input(&input, 20, 10);
    let output = dir.path("out.gif");
    let out = run(&[
        "apply",
        s(&input),
        "-o",
        s(&output),
        "--upscale",
        "3",
        "--seed",
        "5",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("60x30, 2 colours, 1-bit"));
    assert_eq!(
        image::open(&output).unwrap().to_rgba8().dimensions(),
        (60, 30)
    );
}

#[test]
fn reports_errors_with_a_nonzero_exit() {
    let dir = Scratch::new("errors");
    let input = dir.path("input.png");
    write_input(&input, 8, 8);
    let cases: [(&[&str], &str); 4] = [
        (&["apply", "nope.png", "-o", "x.png"], "nope.png"),
        (
            &[
                "apply",
                s(&input),
                "--look",
                "missing.ditherlook",
                "-o",
                "x.png",
            ],
            "missing.ditherlook",
        ),
        (
            &["apply", s(&input), "-o", "x.unknown"],
            "unknown output format",
        ),
        (
            &["apply", s(&input), "-o", "x.png", "--upscale", "0"],
            "at least 1",
        ),
    ];
    for (args, needle) in cases {
        let out = run(args);
        assert!(!out.status.success(), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("error:") && err.contains(needle),
            "{args:?}: {err}"
        );
    }
}

#[test]
fn palette_commands() {
    let out = run(&["palette", "list"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && text.contains("gameboy") && text.contains("web-safe"));

    let dir = Scratch::new("palette");
    let input = dir.path("input.png");
    write_input(&input, 32, 32);
    for method in ["median-cut", "octree", "kmeans"] {
        let out = run(&["palette", "extract", s(&input), "-n", "5", "-m", method]);
        assert!(out.status.success(), "{method}");
        let lines: Vec<_> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(String::from)
            .collect();
        assert!(!lines.is_empty() && lines.len() <= 5, "{method}: {lines:?}");
        assert!(
            lines
                .iter()
                .all(|l| l.len() == 6 && u32::from_str_radix(l, 16).is_ok()),
            "{lines:?}"
        );
    }
    assert!(!run(&["palette", "extract", s(&input), "-n", "1"])
        .status
        .success());
}

#[test]
fn palette_files_extract_convert_and_apply() {
    let dir = Scratch::new("palfiles");
    let input = dir.path("input.png");
    write_input(&input, 32, 32);

    // Extract straight to each file format, then convert between them.
    let gpl = dir.path("p.gpl");
    let out = run(&["palette", "extract", s(&input), "-n", "6", "-o", s(&gpl)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("colours)"));
    let extracted = ditherr_io::read_palette(&gpl).unwrap();
    assert!(!extracted.colors.is_empty() && extracted.colors.len() <= 6);
    assert_eq!(extracted.name, "input");

    let mut previous = gpl.clone();
    for ext in ["hex", "pal", "act", "gpl"] {
        let next = dir.path(&format!("c.{ext}"));
        let out = run(&["palette", "convert", s(&previous), s(&next)]);
        assert!(
            out.status.success(),
            "{ext}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            ditherr_io::read_palette(&next).unwrap().colors,
            extracted.colors,
            "{ext}"
        );
        previous = next;
    }

    // `apply --palette` is the same as putting the colours in the look.
    let output = dir.path("out.png");
    let out = run(&["apply", s(&input), "--palette", s(&gpl), "-o", s(&output)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let look = ditherr_core::Look {
        palette: ditherr_core::PaletteSpec::Custom(extracted.colors),
        ..ditherr_core::Look::default()
    };
    let src = ditherr_io::read_image(&input).unwrap();
    let expected = ditherr_core::render(&src, &look).unwrap().upscale(1);
    assert!(ditherr_io::read_image(&output).unwrap() == expected.to_rgba8());

    // Bad inputs fail with the file named.
    for args in [
        vec!["palette", "convert", "missing.gpl", s(&dir.path("x.hex"))],
        vec!["palette", "convert", s(&gpl), "x.png"],
        vec!["apply", s(&input), "--palette", "nope.hex", "-o", "x.png"],
    ] {
        let out = run(&args);
        assert!(!out.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("error:"),
            "{args:?}"
        );
    }
}
