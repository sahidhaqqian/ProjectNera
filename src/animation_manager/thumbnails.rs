//! Small cached images, independently owned from live VRM assets.
//! Read just the JSON and embedded thumbnail range; never decode a model here.
use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fs::File,
    hash::{Hash, Hasher},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};

#[derive(Default)]
pub(super) struct ThumbnailIo {
    paths: HashMap<String, Option<PathBuf>>,
    read: Option<(String, Task<(Option<PathBuf>, Option<Image>)>)>,
    writes: Vec<Task<()>>,
}

impl ThumbnailIo {
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

    pub fn request(&mut self, key: &str) {
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
                let image = cache
                    .as_deref()
                    .and_then(|path| {
                        std::fs::read(path)
                            .ok()
                            .and_then(|bytes| decode(&bytes, "image/png"))
                    })
                    .or_else(|| embedded_thumbnail(&source).ok().flatten());
                (cache, image)
            }),
        ));
    }

    pub fn poll(&mut self) -> Option<(String, Option<Image>)> {
        let (_, task) = self.read.as_mut()?;
        let (path, image) = block_on(poll_once(task))?;
        let (key, _) = self.read.take()?;
        self.paths.insert(key.clone(), path);
        Some((key, image))
    }

    pub fn store(&mut self, key: &str, image: Image) {
        let Some(Some(path)) = self.paths.get(key) else {
            return;
        };
        if self.writes.len() >= 2 {
            return;
        }
        let path = path.clone();
        self.writes.push(IoTaskPool::get().spawn(async move {
            let Ok(image) = image.try_into_dynamic() else {
                return;
            };
            let Some(directory) = path.parent() else {
                return;
            };
            if std::fs::create_dir_all(directory).is_err() {
                return;
            }
            let temporary = path.with_extension(format!("{}.tmp.png", std::process::id()));
            if image.thumbnail(256, 320).save(&temporary).is_ok() {
                let _ = std::fs::rename(&temporary, &path);
            }
            let _ = std::fs::remove_file(temporary);
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
pub(super) fn remove_cached_model(path: &str) {
    let prefix = format!("{:016x}-", key_hash(&format!("model/{path}")));
    IoTaskPool::get()
        .spawn(async move {
            if let Ok(files) = std::fs::read_dir(cache_directory()) {
                for file in files.flatten() {
                    if file.file_name().to_string_lossy().starts_with(&prefix) {
                        let _ = std::fs::remove_file(file.path());
                    }
                }
            }
        })
        .detach();
}

fn decode(bytes: &[u8], mime: &str) -> Option<Image> {
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
    let small = reader.decode().ok()?.thumbnail(256, 320);
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
