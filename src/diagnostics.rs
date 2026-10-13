//! Local, bounded per-process diagnostics, also available without a console.
use bevy::{
    log::{BoxedLayer, tracing_subscriber},
    prelude::*,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
const LIMIT: u64 = 2 * 1024 * 1024;

fn active_log(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        if let Some(pid) = name
            .strip_prefix("nera-")
            .and_then(|s| s.strip_suffix(".log"))
            .and_then(|s| s.parse::<u32>().ok())
        {
            if let Ok(handle) =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            {
                let _ = unsafe { CloseHandle(handle) };
                return true;
            }
        }
    }
    false
}
struct LogFile {
    file: File,
    size: u64,
}
#[derive(Clone)]
struct Writer(Arc<Mutex<LogFile>>);
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let original_len = bytes.len();
        let mut start = original_len.saturating_sub(LIMIT as usize);
        // Preserve a valid UTF-8 tail even for one oversized diagnostic event.
        while original_len > LIMIT as usize && start < original_len && bytes[start] & 0xc0 == 0x80 {
            start += 1;
        }
        let bytes = &bytes[start..];
        let mut log = self
            .0
            .lock()
            .map_err(|_| io::Error::other("diagnostic lock poisoned"))?;
        if log.size + bytes.len() as u64 > LIMIT {
            log.file.set_len(0)?;
            log.file.seek(SeekFrom::Start(0))?;
            log.size = 0;
        }
        let count = log.file.write(bytes)?;
        log.size += count as u64;
        Ok(start + count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("diagnostic lock poisoned"))?
            .file
            .flush()
    }
}
pub(crate) fn folder() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ProjectNera/logs")
}

pub(crate) fn startup_failure(message: &str) {
    error!(
        "Nera could not start: {message} | logs={}",
        folder().display()
    );
    #[cfg(target_os = "windows")]
    {
        use windows::{
            Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
            core::PCWSTR,
        };
        let text: Vec<_> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<_> = "Project Nera - startup error"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let _ = unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}
pub(crate) fn layer(_: &mut App) -> Option<BoxedLayer> {
    let folder = folder();
    if let Err(error) = fs::create_dir_all(&folder) {
        startup_failure(&format!(
            "Cannot create diagnostics folder {}: {error}",
            folder.display()
        ));
        return None;
    }
    let path = folder.join(format!("nera-{}.log", std::process::id()));
    let mut previous: Vec<_> = fs::read_dir(&folder)
        .ok()?
        .flatten()
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("nera-")
                && !active_log(&e.file_name().to_string_lossy())
                && e.path().extension().is_some_and(|s| s == "log")
        })
        .collect();
    previous.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    for entry in previous.iter().take(previous.len().saturating_sub(4)) {
        let _ = fs::remove_file(entry.path());
    }
    // Windows append-only handles cannot truncate. A per-process mutex owns
    // all writes, so a normal writable handle with an explicit end position is
    // both append-safe here and capable of rotation.
    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            startup_failure(&format!(
                "Cannot open diagnostics file {}: {error}",
                path.display()
            ));
            return None;
        }
    };
    let size = file.seek(SeekFrom::End(0)).ok()?;
    let writer = Writer(Arc::new(Mutex::new(LogFile { file, size })));
    let panic_writer = writer.clone();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let mut writer = panic_writer.clone();
        let _ = writeln!(writer, "FATAL panic: {panic}");
        let _ = writer.flush();
        previous_hook(panic);
    }));
    Some(Box::new(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(move || writer.clone()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panic_probe() {
        if std::env::var_os("NERA_TEST_PANIC_LOG").is_some() {
            let mut app = App::new();
            app.add_plugins(bevy::log::LogPlugin {
                custom_layer: layer,
                ..default()
            });
            panic!("audit diagnostic panic probe");
        }
    }
    #[test]
    fn panic_is_logged_without_swallowing_the_failure() {
        let root = std::env::temp_dir().join(format!("nera-panic-log-{}", std::process::id()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "diagnostics::tests::panic_probe", "--nocapture"])
            .env("NERA_TEST_PANIC_LOG", "1")
            .env("LOCALAPPDATA", &root)
            .output()
            .unwrap();
        assert!(!output.status.success(), "panic must remain a failure");
        let folder = root.join("ProjectNera/logs");
        let entry = fs::read_dir(&folder).unwrap().next().unwrap().unwrap();
        let log = fs::read_to_string(entry.path()).unwrap();
        assert!(log.contains("FATAL panic:") && log.contains("audit diagnostic panic probe"));
        fs::remove_file(entry.path()).unwrap();
        fs::remove_dir(folder).unwrap();
        fs::remove_dir(root.join("ProjectNera")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn diagnostic_file_remains_bounded_and_retains_new_errors() {
        let path =
            std::env::temp_dir().join(format!("nera-bounded-log-{}.txt", std::process::id()));
        File::create(&path).unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        let mut writer = Writer(Arc::new(Mutex::new(LogFile { file, size: 0 })));
        writer.write_all(&vec![b'x'; LIMIT as usize - 4]).unwrap();
        writer.write_all(b"new actionable error").unwrap();
        writer.flush().unwrap();
        assert!(path.metadata().unwrap().len() <= LIMIT);
        assert_eq!(fs::read(&path).unwrap(), b"new actionable error");
        writer.write_all(&vec![b'z'; LIMIT as usize + 100]).unwrap();
        writer.flush().unwrap();
        assert_eq!(path.metadata().unwrap().len(), LIMIT);
        writer.write_all(b"latest failure").unwrap();
        writer.flush().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"latest failure");
        drop(writer);
        fs::remove_file(path).unwrap();
    }
}
