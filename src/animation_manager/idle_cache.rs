//! Full-duration flipbooks: compressed frames stay in RAM; only current/next frames decode.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    path::Path,
    sync::Arc,
};

pub(super) const FPS: f64 = 8.0;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn recording_seconds(clip_seconds: f64) -> f64 {
    if clip_seconds.is_finite() && clip_seconds > 0.0 {
        (30.0 / clip_seconds).ceil().max(1.0) * clip_seconds
    } else {
        30.0
    }
}

pub(super) fn required_seconds() -> f64 {
    recording_seconds(
        super::combine::duration(&crate::nera_asset_file(
            "animations/animasi_basic/idle.vrma",
        ))
        .unwrap_or(30.0) as f64,
    )
}

fn clip_identity() -> Option<String> {
    let metadata = crate::nera_asset_file("animations/animasi_basic/idle.vrma")
        .metadata()
        .ok()?;
    Some(format!(
        "{}-{}",
        metadata.len(),
        metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    ))
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u8,
    fps: f64,
    duration: f64,
    clip: String,
    lengths: Vec<usize>,
}

pub(super) struct Frames {
    bytes: Vec<u8>,
    ranges: Vec<std::ops::Range<usize>>,
}

impl Frames {
    #[cfg(test)]
    pub fn for_test() -> Arc<Self> {
        let image = Image::from_dynamic(
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                256,
                320,
                image::Rgba([80, 100, 120, 255]),
            )),
            true,
            RenderAssetUsages::MAIN_WORLD,
        );
        let encoded = encode(image).unwrap();
        let size = encoded.len();
        Arc::new(Self {
            bytes: encoded.repeat(240),
            ranges: (0..240).map(|i| i * size..(i + 1) * size).collect(),
        })
    }
    pub fn len(&self) -> usize {
        self.ranges.len()
    }
    pub fn bytes(&self) -> usize {
        self.bytes.len()
    }
    pub fn load(path: &Path) -> Option<Arc<Self>> {
        let file = std::fs::File::open(path.with_extension("idle-v2.json")).ok()?;
        if file.metadata().ok()?.len() > 64 * 1024 {
            return None;
        }
        let manifest: Manifest = serde_json::from_reader(file.take(64 * 1024)).ok()?;
        if manifest.version != 2
            || manifest.fps != FPS
            || !manifest.duration.is_finite()
            || manifest.duration < 30.0
            || (manifest.duration * FPS).ceil() as usize != manifest.lengths.len()
            || manifest.clip != clip_identity()?
            || manifest.lengths.len() < 240
            || manifest.lengths.len() > 4096
        {
            return None;
        }
        let mut end = 0usize;
        let mut ranges = Vec::with_capacity(manifest.lengths.len());
        for length in manifest.lengths {
            if length == 0 || length > 256 * 1024 {
                return None;
            }
            let start = end;
            end = end.checked_add(length)?;
            if end as u64 > MAX_BYTES {
                return None;
            }
            ranges.push(start..end);
        }
        let file = std::fs::File::open(path.with_extension("idle-v2.frames")).ok()?;
        if file.metadata().ok()?.len() != end as u64 {
            return None;
        }
        let mut bytes = Vec::with_capacity(end);
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() != end {
            return None;
        }
        let frames = Arc::new(Self { bytes, ranges });
        frames.decode(0)?;
        Some(frames)
    }
    fn decode(&self, index: usize) -> Option<Image> {
        let mut reader = image::ImageReader::with_format(
            std::io::Cursor::new(&self.bytes[self.ranges.get(index)?.clone()]),
            image::ImageFormat::Jpeg,
        );
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(256);
        limits.max_image_height = Some(320);
        limits.max_alloc = Some(2 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().ok()?;
        if image.width() != 256 || image.height() != 320 {
            return None;
        }
        Some(Image::from_dynamic(
            image,
            true,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        ))
    }
    pub fn first(&self) -> Option<Image> {
        self.decode(0)
    }
}

pub(super) fn encode(frame: Image) -> Option<Vec<u8>> {
    let image = frame.try_into_dynamic().ok()?.to_rgb8();
    if image.dimensions() != (256, 320) {
        return None;
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 88)
        .encode_image(&image)
        .ok()?;
    Some(bytes)
}

pub(super) fn store(path: &Path, frames: Vec<Vec<u8>>, duration: f64) -> io::Result<()> {
    if !duration.is_finite()
        || duration < 30.0
        || (duration * FPS).ceil() as usize != frames.len()
        || frames.len() < 240
        || frames.len() > 4096
        || frames
            .iter()
            .any(|frame| frame.is_empty() || frame.len() > 256 * 1024)
        || frames.iter().map(Vec::len).sum::<usize>() as u64 > MAX_BYTES
    {
        return Err(io::Error::other("Invalid idle frame cache"));
    }
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Missing cache parent"))?,
    )?;
    let data = path.with_extension("idle-v2.frames");
    let metadata = path.with_extension("idle-v2.json");
    let temporary = path.with_extension(format!("{}.idle.tmp", std::process::id()));
    let mut output = std::fs::File::create(&temporary)?;
    use std::io::Write;
    for frame in &frames {
        output.write_all(frame)?;
    }
    output.flush()?;
    drop(output);
    std::fs::rename(&temporary, data)?;
    let manifest = Manifest {
        version: 2,
        fps: FPS,
        duration,
        clip: clip_identity().ok_or_else(|| io::Error::other("Missing idle clip"))?,
        lengths: frames.iter().map(Vec::len).collect(),
    };
    std::fs::write(&temporary, serde_json::to_vec(&manifest)?)?;
    std::fs::rename(temporary, metadata)
}

pub(super) struct Player {
    frames: Arc<Frames>,
    displayed: usize,
    start: Option<f64>,
    next: Option<(usize, Image)>,
    pending: Option<Task<(usize, Option<Image>)>>,
}

impl Player {
    pub fn new(frames: Arc<Frames>) -> Self {
        Self {
            frames,
            displayed: 0,
            start: None,
            next: None,
            pending: None,
        }
    }
    pub fn index(&self) -> usize {
        self.displayed
    }
    pub fn count(&self) -> usize {
        self.frames.len()
    }
    pub fn bytes(&self) -> usize {
        self.frames.bytes()
    }
    pub fn update(&mut self, now: f64) -> Option<Image> {
        let start = *self.start.get_or_insert(now);
        if let Some(task) = self.pending.as_mut()
            && let Some((index, image)) = block_on(poll_once(task))
        {
            self.pending = None;
            self.next = image.map(|image| (index, image));
        }
        let desired = (((now - start).max(0.0) * FPS).floor() as usize) % self.frames.len();
        let mut image = None;
        if desired != self.displayed && self.next.is_some() {
            let (index, ready) = self.next.take().unwrap();
            self.displayed = index;
            image = Some(ready);
        }
        if self.pending.is_none() {
            let index = if desired != self.displayed {
                desired
            } else {
                (self.displayed + 1) % self.frames.len()
            };
            if self.next.as_ref().is_none_or(|(ready, _)| *ready != index) {
                let frames = self.frames.clone();
                self.pending =
                    Some(IoTaskPool::get().spawn(async move { (index, frames.decode(index)) }));
            }
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_that_finishes_after_its_deadline_still_advances_the_preview() {
        IoTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let frames = Frames::for_test();
        let mut player = Player::new(frames.clone());
        player.start = Some(0.0);
        player.next = Some((1, frames.decode(1).unwrap()));
        assert!(player.update(2.0 / FPS).is_some());
        assert_eq!(
            player.index(),
            1,
            "late frame is displayed instead of dropped"
        );
        assert!(
            player.pending.is_some(),
            "current desired frame is requested next"
        );
    }
    #[test]
    fn recording_covers_thirty_seconds_and_whole_idle_cycles() {
        assert_eq!(recording_seconds(12.0), 36.0);
        assert_eq!(recording_seconds(45.0), 45.0);
        assert_eq!(recording_seconds(f64::NAN), 30.0);
    }
    #[test]
    fn playback_visits_the_full_timeline_and_wraps_with_bounded_decode_state() {
        IoTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let frames = Frames::for_test();
        let mut player = Player::new(frames.clone());
        assert!(player.update(0.0).is_none());
        for index in 1..=240 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while player.index() != index % 240 {
                let _ = player.update(index as f64 / FPS);
                assert!(
                    std::time::Instant::now() < deadline,
                    "frame {index} did not decode"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(player.count(), 240);
            assert!(
                Arc::strong_count(&frames) <= 3,
                "only player and one decoder may retain frames"
            );
        }
        drop(player);
        for _ in 0..100 {
            if Arc::strong_count(&frames) == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            Arc::strong_count(&frames),
            1,
            "closing releases frame/decode state"
        );
    }
    #[test]
    fn full_sequence_persists_compressed_and_corrupt_data_is_rejected() {
        let directory =
            std::env::temp_dir().join(format!("nera-idle-sequence-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.png");
        let image = Image::from_dynamic(
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                256,
                320,
                image::Rgba([128, 64, 0, 255]),
            )),
            true,
            RenderAssetUsages::MAIN_WORLD,
        );
        let encoded = encode(image).unwrap();
        store(&path, vec![encoded; 240], 30.0).unwrap();
        let loaded = Frames::load(&path).unwrap();
        assert_eq!(loaded.len(), 240);
        assert!(loaded.bytes() < 4 * 1024 * 1024);
        assert_eq!(loaded.first().unwrap().width(), 256);
        std::fs::write(path.with_extension("idle-v2.frames"), b"truncated").unwrap();
        assert!(Frames::load(&path).is_none());
        std::fs::remove_file(path.with_extension("idle-v2.frames")).unwrap();
        std::fs::remove_file(path.with_extension("idle-v2.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
