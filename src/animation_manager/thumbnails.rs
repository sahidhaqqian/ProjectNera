//! Small cached images, independently owned from live VRM assets.
//! Read just the JSON and embedded thumbnail range; never decode a model here.
use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fs::File,
    hash::{Hash, Hasher},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use super::idle_cache::Frames;
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};

type CachedPreview = (Option<PathBuf>, Option<Image>, Option<Arc<Frames>>);

pub(super) const IDLE_FRAMES: usize = 10;
const ATLAS_COLUMNS: u32 = 5;
const FRAME_WIDTH: u32 = 256;
const FRAME_HEIGHT: u32 = 320;

pub(super) fn is_idle_atlas(image: &Image) -> bool {
    image.width() == FRAME_WIDTH * ATLAS_COLUMNS && image.height() == FRAME_HEIGHT * 2
}

pub(super) fn frame_rect(frame: usize) -> Rect {
    let frame = (frame % IDLE_FRAMES) as u32;
    let x = (frame % ATLAS_COLUMNS) * FRAME_WIDTH;
    let y = (frame / ATLAS_COLUMNS) * FRAME_HEIGHT;
    Rect::new(
        x as f32,
        y as f32,
        (x + FRAME_WIDTH) as f32,
        (y + FRAME_HEIGHT) as f32,
    )
}

#[cfg(test)]
pub(super) fn pack_frames(frames: &[image::RgbaImage]) -> Option<Image> {
    if frames.len() != IDLE_FRAMES
        || frames
            .iter()
            .any(|frame| frame.dimensions() != (FRAME_WIDTH, FRAME_HEIGHT))
    {
        return None;
    }
    let mut atlas = image::RgbaImage::new(FRAME_WIDTH * ATLAS_COLUMNS, FRAME_HEIGHT * 2);
    for (index, frame) in frames.iter().enumerate() {
        image::imageops::replace(
            &mut atlas,
            frame,
            ((index as u32 % ATLAS_COLUMNS) * FRAME_WIDTH) as i64,
            ((index as u32 / ATLAS_COLUMNS) * FRAME_HEIGHT) as i64,
        );
    }
    Some(Image::from_dynamic(
        image::DynamicImage::ImageRgba8(atlas),
        true,
        RenderAssetUsages::MAIN_WORLD,
    ))
}

#[derive(Default)]
pub(super) struct ThumbnailIo {
    paths: HashMap<String, Option<PathBuf>>,
    read: Option<(String, Task<CachedPreview>)>,
    writes: Vec<Task<()>>,
}

impl ThumbnailIo {
    pub fn idle(&self) -> bool {
        self.read.is_none() && self.writes.is_empty()
    }
    #[cfg(test)]
    pub fn missing_for_test(&mut self, key: String) {
        self.paths.insert(key, None);
    }
    pub fn retain(&mut self, needed: &HashSet<String>) {
        self.paths.retain(|key, _| needed.contains(key));
        if self
            .read
            .as_ref()
            .is_some_and(|(key, _)| !needed.contains(key))
        {
            self.read = None;
        }
        self.writes
            .retain_mut(|task| block_on(poll_once(task)).is_none());
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.read = None;
        self.writes.clear();
    }

    pub fn attempted(&self, key: &str) -> bool {
        self.paths.contains_key(key)
    }

    pub fn request(&mut self, key: &str, render_only: bool) {
        if self.read.is_some() || self.attempted(key) {
            return;
        }
        let Some(source) = key
            .strip_prefix("model/")
            .and_then(crate::character::preview_model_file)
        else {
            self.paths.insert(key.into(), None);
            return;
        };
        let path = key.to_owned();
        self.read = Some((
            path.clone(),
            IoTaskPool::get().spawn(async move {
                let cache = cache_file(&path, &source).ok();
                if render_only {
                    return (cache, None, None);
                }
                if let Some(frames) = cache.as_deref().and_then(Frames::load) {
                    let image = frames.first();
                    return (cache, image, Some(frames));
                }
                let image = cache
                    .as_ref()
                    .and_then(|path| read_cached(&path.with_extension("idle.png")))
                    .or_else(|| cache.as_deref().and_then(read_cached))
                    .or_else(|| {
                        let image = embedded_thumbnail(&source).ok().flatten()?;
                        if let Some(path) = &cache {
                            let _ = save_image(path, image.clone());
                        }
                        Some(image)
                    });
                (cache, image, None)
            }),
        ));
    }

    pub fn poll(&mut self) -> Option<(String, Option<Image>, Option<Arc<Frames>>)> {
        let (_, task) = self.read.as_mut()?;
        let (path, image, frames) = block_on(poll_once(task))?;
        let (key, _) = self.read.take()?;
        self.paths.insert(key.clone(), path);
        Some((key, image, frames))
    }

    pub fn store_sequence(&mut self, key: &str, frames: Vec<Vec<u8>>, duration: f64) {
        let Some(Some(path)) = self.paths.get(key) else {
            return;
        };
        if self.writes.len() >= 2 {
            return;
        }
        let path = path.clone();
        self.writes.push(IoTaskPool::get().spawn(async move {
            if let Err(error) = super::idle_cache::store(&path, frames, duration) {
                warn!("Could not save character idle preview: {error}");
            }
        }));
    }
}

fn cache_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("io.github.sahidhaqqi.projectnera")
        .join("thumbnails-v1")
}

fn key_hash(key: &str) -> u64 {
    let mut hash = DefaultHasher::new();
    key.hash(&mut hash);
    hash.finish()
}

fn cache_file(key: &str, source: &Path) -> io::Result<PathBuf> {
    let metadata = source.metadata()?;
    let stamp = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(cache_directory().join(format!(
        "{:016x}-{}-{stamp}.png",
        key_hash(key),
        metadata.len()
    )))
}

/// Called only when a roster entry disappears, never for ordinary scrolling.
pub(crate) fn remove_cached_model(path: &str) {
    let prefix = format!("{:016x}-", key_hash(&format!("model/{path}")));
    // Complete deletion before another import can write the same cache identity.
    if let Ok(files) = std::fs::read_dir(cache_directory()) {
        for file in files.flatten() {
            if file.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }
}

fn read_cached(path: &Path) -> Option<Image> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    decode_sized(
        &bytes,
        "image/png",
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".idle.png")),
    )
}

fn save_image(path: &Path, image: Image) -> io::Result<()> {
    let image = image.try_into_dynamic().map_err(|_| invalid())?;
    std::fs::create_dir_all(path.parent().ok_or_else(invalid)?)?;
    let temporary = path.with_extension(format!("{}.tmp.png", std::process::id()));
    let image =
        if image.width() == FRAME_WIDTH * ATLAS_COLUMNS && image.height() == FRAME_HEIGHT * 2 {
            image
        } else {
            image.thumbnail(FRAME_WIDTH, FRAME_HEIGHT)
        };
    let result = image
        .save(&temporary)
        .map_err(io::Error::other)
        .and_then(|_| std::fs::rename(&temporary, path));
    let _ = std::fs::remove_file(temporary);
    result
}

/// Import-only generation. The gallery itself never creates a rig on a cache miss.
pub(crate) fn prepare(asset_path: &str) -> Result<(), String> {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let source =
        crate::character::preview_model_file(asset_path).ok_or("Invalid preview source")?;
    let cache = cache_file(&format!("model/{asset_path}"), &source).map_err(|e| e.to_string())?;
    if Frames::load(&cache).is_some() {
        return Ok(());
    }
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("--animation-manager-worker")
        .arg(format!("--generate-character-preview={asset_path}"))
        .env_remove("NERA_GALLERY_SMOKE_TEST")
        .env_remove("NERA_GALLERY_HOST_SMOKE_TEST")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let deadline = Duration::from_secs_f64(super::idle_cache::required_seconds() * 2.0 + 60.0);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Thumbnail generation timed out".into());
            }
        }
    }
    if Frames::load(&cache).is_some() {
        Ok(())
    } else {
        Err("No usable thumbnail generated; import remains available".into())
    }
}

fn decode(bytes: &[u8], mime: &str) -> Option<Image> {
    decode_sized(bytes, mime, false)
}

fn decode_sized(bytes: &[u8], mime: &str, animated: bool) -> Option<Image> {
    if bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    let format = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        _ => return None,
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let small = if animated {
        if decoded.width() != FRAME_WIDTH * ATLAS_COLUMNS || decoded.height() != FRAME_HEIGHT * 2 {
            return None;
        }
        decoded
    } else {
        decoded.thumbnail(FRAME_WIDTH, FRAME_HEIGHT)
    };
    let pixels = small.to_rgba8();
    let first = pixels.pixels().next()?;
    if pixels.pixels().all(|pixel| pixel == first) {
        return None;
    }
    Some(Image::from_dynamic(
        small,
        true,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid VRM thumbnail")
}

fn embedded_thumbnail(path: &Path) -> io::Result<Option<Image>> {
    let mut file = File::open(path)?;
    let mut header = [0; 20];
    file.read_exact(&mut header)?;
    if &header[..4] != b"glTF" || header[4..8] != 2u32.to_le_bytes() || &header[16..20] != b"JSON" {
        return Err(invalid());
    }
    let length = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if length > 4 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut json = vec![0; length];
    file.read_exact(&mut json)?;
    let document: serde_json::Value = serde_json::from_slice(&json).map_err(|_| invalid())?;
    let Some(index) = document
        .pointer("/extensions/VRMC_vrm/meta/thumbnailImage")
        .and_then(|v| v.as_u64())
    else {
        return Ok(None);
    };
    let image = &document["images"][index as usize];
    let view_index = image["bufferView"].as_u64().ok_or_else(invalid)?;
    let view = &document["bufferViews"][view_index as usize];
    if view["buffer"].as_u64() != Some(0) {
        return Err(invalid());
    }
    let offset = view["byteOffset"].as_u64().unwrap_or(0);
    let size = view["byteLength"].as_u64().ok_or_else(invalid)?;
    let mime = image["mimeType"].as_str().ok_or_else(invalid)?;
    let mut binary = [0; 8];
    file.read_exact(&mut binary)?;
    let binary_size = u64::from(u32::from_le_bytes(binary[..4].try_into().unwrap()));
    if binary[4..] != [b'B', b'I', b'N', 0]
        || size > 8 * 1024 * 1024
        || offset.checked_add(size).is_none_or(|end| end > binary_size)
    {
        return Err(invalid());
    }
    file.seek(SeekFrom::Current(offset as i64))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    Ok(decode(&bytes, mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_idle_frames_round_trip_as_an_atlas_and_loop_without_scaling_frames() {
        let frames: Vec<_> = (0..IDLE_FRAMES)
            .map(|frame| {
                image::RgbaImage::from_pixel(256, 320, image::Rgba([frame as u8, 64, 128, 255]))
            })
            .collect();
        assert!(pack_frames(&frames[..9]).is_none());
        let atlas = pack_frames(&frames).unwrap();
        assert!(is_idle_atlas(&atlas));
        let atlas = atlas.try_into_dynamic().unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        atlas.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let decoded = decode_sized(bytes.get_ref(), "image/png", true)
            .unwrap()
            .try_into_dynamic()
            .unwrap()
            .to_rgba8();
        for frame in 0..IDLE_FRAMES {
            let rect = frame_rect(frame);
            assert_eq!(
                decoded.get_pixel(rect.min.x as u32 + 10, rect.min.y as u32 + 10)[0],
                frame as u8
            );
            assert_eq!(rect.width(), 256.0);
            assert_eq!(rect.height(), 320.0);
        }
        assert_eq!(frame_rect(10), frame_rect(0));
    }

    #[test]
    fn removing_cached_model_deletes_all_its_versions_only() {
        let key = format!(
            "characters://model-cache-removal-{}.vrm",
            std::process::id()
        );
        let other = format!(
            "characters://model-cache-unrelated-{}.vrm",
            std::process::id()
        );
        let directory = cache_directory();
        std::fs::create_dir_all(&directory).unwrap();
        let prefix = format!("{:016x}-", key_hash(&format!("model/{key}")));
        let other_file = directory.join(format!(
            "{:016x}-test.png",
            key_hash(&format!("model/{other}"))
        ));
        let versions = [
            directory.join(format!("{prefix}old.png")),
            directory.join(format!("{prefix}new.png")),
        ];
        for path in versions.iter().chain(std::iter::once(&other_file)) {
            std::fs::write(path, b"owned test cache").unwrap();
        }
        remove_cached_model(&key);
        assert!(versions.iter().all(|path| !path.exists()));
        assert!(other_file.exists());
        remove_cached_model(&key);
        std::fs::remove_file(other_file).unwrap();
    }

    #[test]
    fn embedded_default_thumbnail_is_small_without_loading_a_vrm_scene() {
        let path =
            crate::character::preview_model_file(crate::character::DEFAULT_CHARACTER).unwrap();
        let thumbnail = embedded_thumbnail(&path).unwrap().unwrap();
        assert!(thumbnail.width() <= 256 && thumbnail.height() <= 320);
        assert_eq!(thumbnail.asset_usage, RenderAssetUsages::RENDER_WORLD);
        assert!(thumbnail.data.unwrap().len() <= 256 * 320 * 4);
        assert!(crate::character::preview_model_file("characters://../outside.vrm").is_none());
    }

    #[test]
    fn corrupt_or_oversized_thumbnail_degrades_to_render_fallback() {
        assert!(decode(b"invalid png", "image/png").is_none());
        let mut bytes = vec![0; 24];
        bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&bytes, "image/png").is_none());
        assert!(decode(&bytes, "image/svg+xml").is_none());
        let blank = image::DynamicImage::new_rgba8(256, 320);
        let mut encoded = std::io::Cursor::new(Vec::new());
        blank
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        assert!(decode(encoded.get_ref(), "image/png").is_none());
    }

    #[test]
    fn cache_identity_changes_with_source_and_thumbnail_decoder_preserves_content() {
        let directory =
            std::env::temp_dir().join(format!("nera-thumbnail-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.vrm");
        std::fs::write(&source, b"first version").unwrap();
        let first = cache_file("model/test", &source).unwrap();
        std::fs::write(&source, b"second version, different content").unwrap();
        assert_ne!(first, cache_file("model/test", &source).unwrap());
        let mut rgba = image::RgbaImage::new(512, 640);
        for (x, y, pixel) in rgba.enumerate_pixels_mut() {
            *pixel = image::Rgba([x as u8, y as u8, 64, 255]);
        }
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(encoded.get_ref(), "image/png").unwrap();
        assert_eq!((decoded.width(), decoded.height()), (256, 320));
        assert!(
            decoded
                .data
                .unwrap()
                .chunks_exact(4)
                .any(|pixel| pixel[0] != 0)
        );
        std::fs::remove_file(source).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
