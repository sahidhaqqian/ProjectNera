//! Windows startup registration for Project Nera.
//!
//! The registry itself is the source of truth. This module intentionally does
//! not persist a duplicate `start_with_windows` boolean in Bevy settings.

#[cfg(target_os = "windows")]
mod imp {
    use std::{ffi::{c_void, OsStr}, os::windows::ffi::OsStrExt};

    use windows::{
        core::w,
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
            System::Registry::{
                HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW,
                RegGetValueW, RegSetKeyValueW,
            },
        },
    };

    const VALUE_NAME: &str = "Project Nera";

    fn current_command() -> Result<Vec<u16>, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("cannot resolve current executable: {error}"))?;
        let command = format!("\"{}\"", executable.display());
        Ok(OsStr::new(&command).encode_wide().chain(Some(0)).collect())
    }

    pub(super) fn is_enabled() -> Result<bool, String> {
        let mut byte_count = 0u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                w!("Project Nera"),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut byte_count),
            )
        };

        if status == ERROR_SUCCESS {
            return Ok(byte_count > 0);
        }
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            return Ok(false);
        }

        Err(format!("registry query failed with Win32 error {}", status.0))
    }

    pub(super) fn set_enabled(enabled: bool) -> Result<(), String> {
        let status = if enabled {
            let command = current_command()?;
            let byte_count = (command.len() * std::mem::size_of::<u16>()) as u32;
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                    w!("Project Nera"),
                    REG_SZ.0,
                    Some(command.as_ptr().cast::<c_void>()),
                    byte_count,
                )
            }
        } else {
            unsafe {
                RegDeleteKeyValueW(
                    HKEY_CURRENT_USER,
                    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                    w!("Project Nera"),
                )
            }
        };

        if status == ERROR_SUCCESS {
            return Ok(());
        }

        if !enabled && (status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND) {
            return Ok(());
        }

        Err(format!(
            "registry {} failed for {VALUE_NAME} with Win32 error {}",
            if enabled { "write" } else { "delete" },
            status.0
        ))
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn is_enabled() -> Result<bool, String> {
    imp::is_enabled()
}

#[cfg(target_os = "windows")]
pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    imp::set_enabled(enabled)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn is_enabled() -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_enabled(_enabled: bool) -> Result<(), String> {
    Err("Start with Windows is only available on Windows".into())
}
