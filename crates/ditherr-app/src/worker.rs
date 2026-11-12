//! Background rendering. The UI thread never runs the engine: it submits a
//! [`Job`] and picks up the [`Done`] result on a later frame.
//!
//! Jobs are latest-wins. Every [`Worker::submit`] bumps a generation counter and
//! the thread drops any queued job that is no longer current, so dragging a
//! slider renders only the positions the engine can keep up with.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ditherr_core::look::{dither_image, preprocess};
use ditherr_core::{render, ImageF32, Look, Resampler, Rgba8Image};

use crate::info::{self, Stats};

/// Longest edge of the image the live preview works on. Larger sources are
/// shrunk once on load; export always renders the original.
pub const PREVIEW_EDGE: u32 = 2048;
/// Longest edge of gallery thumbnails.
pub const THUMB_EDGE: u32 = 144;

pub struct Job {
    /// Caller-defined slot, echoed back (preview, or a gallery index).
    pub key: usize,
    /// Caller-defined sequence number, echoed back so stale results can be told apart.
    pub tag: u64,
    pub src: Arc<Rgba8Image>,
    pub look: Look,
    /// Also gather the info-panel statistics. They cost a PNG encode, so only
    /// the Studio preview asks for them.
    pub measure: bool,
}

#[derive(Debug)]
pub struct Output {
    /// The result at the source's size, ready to display or encode.
    pub image: Rgba8Image,
    pub colors: usize,
    pub took: Duration,
    /// Present when the job asked to be measured.
    pub stats: Option<Stats>,
}

pub struct Done {
    pub key: usize,
    pub tag: u64,
    pub result: Result<Output, String>,
}

type Repaint = Arc<dyn Fn() + Send + Sync>;

pub struct Worker {
    tx: Sender<(u64, Job)>,
    rx: Receiver<Done>,
    generation: Arc<AtomicU64>,
}

impl Worker {
    /// `repaint` is called whenever a result is ready.
    pub fn new(repaint: Repaint) -> Self {
        let (tx, jobs) = channel::<(u64, Job)>();
        let (done_tx, rx) = channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = Arc::clone(&generation);
        std::thread::Builder::new()
            .name("ditherr-render".into())
            .spawn(move || {
                while let Ok((gen, job)) = jobs.recv() {
                    if gen < current.load(Ordering::Acquire) {
                        continue;
                    }
                    let (key, tag) = (job.key, job.tag);
                    let result = run(&job);
                    if done_tx.send(Done { key, tag, result }).is_err() {
                        break;
                    }
                    repaint();
                }
            })
            .expect("spawn render thread");
        Self { tx, rx, generation }
    }

    /// Starts a new batch of jobs, cancelling every job from earlier batches
    /// that has not started yet. Returns the batch's generation.
    pub fn begin_batch(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Queues a job in the batch returned by [`Worker::begin_batch`].
    pub fn submit(&self, batch: u64, job: Job) {
        // The thread only stops when the app is shutting down.
        let _ = self.tx.send((batch, job));
    }

    /// Results that have arrived since the last call. Results from cancelled
    /// batches can still arrive if they were already running; callers match on
    /// `key` and ignore what they no longer want.
    pub fn poll(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }
}

fn run(job: &Job) -> Result<Output, String> {
    let start = Instant::now();
    // `render` is these two calls; split so the pre-processed image is on hand
    // for the histogram.
    let (rendered, working) = if job.measure {
        let working = preprocess(&job.src, &job.look).map_err(|e| e.to_string())?;
        let rendered = dither_image(&working, &job.look).map_err(|e| e.to_string())?;
        (rendered, Some(working))
    } else {
        (
            render(&job.src, &job.look).map_err(|e| e.to_string())?,
            None,
        )
    };
    let colors = match &rendered {
        ditherr_core::Rendered::Indexed(i) => count_used(&i.indices, i.palette.len()),
        ditherr_core::Rendered::Truecolor(t) => count_truecolor(t),
    };
    // Back to the source's size so the chunky pixels are visible 1:1.
    let upscaled = rendered.upscale(job.look.pixel_scale);
    let stats = working.map(|w| {
        // A failed encode only costs the size readout.
        let png = ditherr_io::encode(&upscaled, ditherr_io::Format::Png).map_or(0, |b| b.len());
        info::analyze(&w, &upscaled, png)
    });
    Ok(Output {
        image: upscaled.to_rgba8(),
        colors,
        took: start.elapsed(),
        stats,
    })
}

fn count_used(indices: &[u8], palette_len: usize) -> usize {
    let mut seen = vec![false; palette_len.max(256)];
    for &i in indices {
        seen[usize::from(i)] = true;
    }
    seen.iter().filter(|&&s| s).count()
}

fn count_truecolor(img: &Rgba8Image) -> usize {
    let mut set = std::collections::HashSet::new();
    for px in img.data.chunks_exact(4) {
        set.insert([px[0], px[1], px[2]]);
    }
    set.len()
}

/// Shrinks `src` so its longest edge is at most `max_edge`; smaller images are
/// returned unchanged.
pub fn fit_within(src: &Rgba8Image, max_edge: u32) -> Rgba8Image {
    let long = src.width.max(src.height);
    if long <= max_edge {
        return src.clone();
    }
    let scale = max_edge as f32 / long as f32;
    let w = ((src.width as f32 * scale).round() as u32).max(1);
    let h = ((src.height as f32 * scale).round() as u32).max(1);
    match ditherr_core::resize::resize(&ImageF32::from_rgba8(src), w, h, Resampler::Lanczos) {
        Ok(img) => img.to_rgba8(),
        Err(_) => src.clone(),
    }
}

/// The look as it should be applied to an image shrunk by `shrink` (`< 1`):
/// the pixel scale shrinks with it so chunky pixels keep their on-screen size.
pub fn scaled_look(look: &Look, shrink: f32) -> Look {
    let mut l = look.clone();
    l.pixel_scale = ((look.pixel_scale as f32 * shrink).round() as u32).max(1);
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn image(w: u32, h: u32) -> Arc<Rgba8Image> {
        let data = (0..w * h)
            .flat_map(|i| [(i % 251) as u8, (i % 13 * 19) as u8, 90, 255])
            .collect();
        Arc::new(Rgba8Image::new(w, h, data).unwrap())
    }

    fn wait_for(worker: &Worker, n: usize) -> Vec<Done> {
        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while out.len() < n && Instant::now() < deadline {
            out.extend(worker.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    #[test]
    fn renders_at_source_size_and_reports_colours() {
        let repaints = Arc::new(Mutex::new(0));
        let r = Arc::clone(&repaints);
        let worker = Worker::new(Arc::new(move || *r.lock().unwrap() += 1));
        let src = image(30, 20);
        let look = Look {
            pixel_scale: 3,
            ..Look::default()
        };
        let batch = worker.begin_batch();
        worker.submit(
            batch,
            Job {
                key: 7,
                tag: 0,
                src,
                look,
                measure: false,
            },
        );
        let done = wait_for(&worker, 1);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].key, 7);
        let out = done[0].result.as_ref().unwrap();
        // 30x20 at scale 3 renders 10x7 and upscales to 30x21.
        assert_eq!((out.image.width, out.image.height), (30, 21));
        assert_eq!(out.colors, 2);
        assert!(*repaints.lock().unwrap() >= 1);
    }

    #[test]
    fn measured_jobs_carry_statistics_and_match_unmeasured_pixels() {
        let worker = Worker::new(Arc::new(|| {}));
        let src = image(40, 24);
        let look = Look {
            pixel_scale: 2,
            ..Look::default()
        };
        let batch = worker.begin_batch();
        for (key, measure) in [(0, false), (1, true)] {
            worker.submit(
                batch,
                Job {
                    key,
                    tag: 0,
                    src: Arc::clone(&src),
                    look: look.clone(),
                    measure,
                },
            );
        }
        let mut done = wait_for(&worker, 2);
        done.sort_by_key(|d| d.key);
        let plain = done[0].result.as_ref().unwrap();
        let measured = done[1].result.as_ref().unwrap();
        assert!(plain.stats.is_none());
        // Splitting the pipeline to get the histogram must not change a pixel.
        assert_eq!(plain.image, measured.image);

        let stats = measured.stats.as_ref().unwrap();
        assert_eq!(stats.colors, measured.colors);
        assert_eq!(stats.histogram.len(), crate::info::HISTOGRAM_BINS);
        // The histogram counts the dithered (pixel-scaled) image: 20x12.
        assert_eq!(stats.histogram.iter().sum::<u32>(), 20 * 12);
        assert_eq!(stats.pixels, 40 * 24);
        assert!(stats.png_bytes > 0);
        let used: u32 = stats.top_colors.iter().map(|c| c.1).sum();
        assert_eq!(u64::from(used), stats.pixels);
    }

    #[test]
    fn invalid_look_comes_back_as_an_error() {
        let worker = Worker::new(Arc::new(|| {}));
        let batch = worker.begin_batch();
        worker.submit(
            batch,
            Job {
                key: 0,
                tag: 0,
                src: image(8, 8),
                look: Look {
                    pixel_scale: 0,
                    ..Look::default()
                },
                measure: false,
            },
        );
        let done = wait_for(&worker, 1);
        assert!(done[0].result.is_err());
    }

    #[test]
    fn stale_batches_are_skipped() {
        let worker = Worker::new(Arc::new(|| {}));
        let big = image(900, 900);
        // The first job keeps the thread busy while the rest queue up.
        let b = worker.begin_batch();
        worker.submit(
            b,
            Job {
                key: 0,
                tag: 0,
                src: Arc::clone(&big),
                look: Look::default(),
                measure: false,
            },
        );
        for key in 1..=5 {
            worker.submit(
                b,
                Job {
                    key,
                    tag: 0,
                    src: image(8, 8),
                    look: Look::default(),
                    measure: false,
                },
            );
        }
        // A new batch cancels queued jobs from the old one.
        let b2 = worker.begin_batch();
        worker.submit(
            b2,
            Job {
                key: 99,
                tag: 0,
                src: image(8, 8),
                look: Look::default(),
                measure: false,
            },
        );
        let mut keys: Vec<usize> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !keys.contains(&99) && Instant::now() < deadline {
            keys.extend(worker.poll().iter().map(|d| d.key));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(keys.contains(&99));
        assert!(
            keys.iter().all(|k| *k == 0 || *k == 99),
            "stale jobs ran: {keys:?}"
        );
    }

    #[test]
    fn fit_within_only_shrinks() {
        let src = image(400, 100);
        assert_eq!(fit_within(&src, 800).width, 400);
        let small = fit_within(&src, 200);
        assert_eq!((small.width, small.height), (200, 50));
    }

    #[test]
    fn scaled_look_keeps_at_least_one_pixel() {
        let l = Look {
            pixel_scale: 4,
            ..Look::default()
        };
        assert_eq!(scaled_look(&l, 0.5).pixel_scale, 2);
        assert_eq!(scaled_look(&l, 0.01).pixel_scale, 1);
    }
}
