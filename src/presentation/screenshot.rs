//! Tray requests cross to Bevy; captures preserve alpha and never change presentation.
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    window::PrimaryWindow,
};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[derive(Message)]
pub(crate) struct ScreenshotRequest;

pub(super) fn install(app: &mut App) {
    app.add_message::<ScreenshotRequest>()
        .add_systems(Update, capture.after(crate::tray::process_tray_commands));
}

pub(super) fn pictures_folder() -> Option<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    if let Ok(value) = unsafe { SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None) } {
        let path = unsafe { value.to_string() }.ok().map(PathBuf::from);
        unsafe { CoTaskMemFree(Some(value.0.cast())) };
        if path.is_some() {
            return path;
        }
    }
    std::env::var_os("USERPROFILE").map(|profile| PathBuf::from(profile).join("Pictures"))
}

fn timestamp() -> String {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format_timestamp(
        now.wYear,
        now.wMonth,
        now.wDay,
        now.wHour,
        now.wMinute,
        now.wSecond,
    )
}

fn format_timestamp(
    year: u16,
    month: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
) -> String {
    format!("Nera_{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

fn reserve(pictures: &Path, name: &str) -> io::Result<(PathBuf, File)> {
    let folder = pictures.join("NeraScreenshots");
    fs::create_dir_all(&folder)?;
    for suffix in 0..10_000 {
        let filename = if suffix == 0 {
            format!("{name}.png")
        } else {
            format!("{name}_{suffix}.png")
        };
        let path = folder.join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "screenshot filename limit reached",
    ))
}

fn capture(
    mut requests: MessageReader<ScreenshotRequest>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    for _ in requests.read() {
        let Ok(window) = windows.single() else {
            warn!("Screenshot skipped: primary window unavailable");
            continue;
        };
        let Some(folder) = pictures_folder() else {
            warn!("Screenshot failed: Pictures folder unavailable");
            continue;
        };
        let (path, mut file) = match reserve(&folder, &timestamp()) {
            Ok(value) => value,
            Err(error) => {
                warn!("Screenshot directory/file creation failed: {error}");
                continue;
            }
        };
        commands
            .spawn(Screenshot::window(window))
            .observe(move |event: On<ScreenshotCaptured>| {
                // Bevy save_to_disk converts to RGB; its screenshot observer
                // foundation is reused here with RGBA for the transparent pet.
                let result = event
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|error| error.to_string())
                    .and_then(|image| {
                        image
                            .write_to(&mut file, image::ImageFormat::Png)
                            .map_err(|error| error.to_string())
                    });
                match result {
                    Ok(()) => info!("Nera screenshot saved: {}", path.display()),
                    Err(error) => {
                        warn!("Screenshot save failed: {error}");
                        let _ = fs::remove_file(&path);
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_timestamp_collision_and_failure() {
        let root =
            std::env::temp_dir().join(format!("nera-screenshot-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let name = format_timestamp(2026, 10, 5, 3, 4, 9);
        assert_eq!(name, "Nera_2026-10-05_03-04-09");
        let (first, file) = reserve(&root, &name).unwrap();
        drop(file);
        fs::write(&first, b"original").unwrap();
        let (second, file) = reserve(&root, &name).unwrap();
        drop(file);
        assert_eq!(
            first,
            root.join("NeraScreenshots").join(format!("{name}.png"))
        );
        assert_eq!(
            second.file_name().unwrap(),
            format!("{name}_1.png").as_str()
        );
        assert_eq!(fs::read(&first).unwrap(), b"original");
        assert!(reserve(&first, &name).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
