//! Shared executable resources; Windows owns these LR_SHARED icon handles.
use bevy::prelude::*;
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            HICON, IMAGE_ICON, LR_SHARED, LoadImageW, SendMessageW, WM_SETICON,
        },
    },
    core::{PCWSTR, Result},
};

pub(crate) fn shared_icon(size: i32) -> Result<HICON> {
    let module = unsafe { GetModuleHandleW(None)? };
    let handle = unsafe {
        LoadImageW(
            Some(HINSTANCE(module.0)),
            PCWSTR(1usize as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_SHARED,
        )?
    };
    Ok(HICON(handle.0))
}

#[derive(Component)]
struct IconApplied;
pub(crate) struct ApplicationIconPlugin;
impl Plugin for ApplicationIconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(First, apply);
    }
}
fn apply(world: &mut World) {
    let pending: Vec<_> = world
        .query_filtered::<Entity, (With<Window>, Without<IconApplied>)>()
        .iter(world)
        .collect();
    for entity in pending {
        let Some(handle) = crate::platform::native_window_handle(entity) else {
            continue;
        };
        let hwnd = HWND(handle.as_raw() as *mut std::ffi::c_void);
        let icons = shared_icon(16).and_then(|small| shared_icon(32).map(|large| (small, large)));
        match icons {
            Ok((small, large)) => unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(0)),
                    Some(LPARAM(small.0 as isize)),
                );
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(1)),
                    Some(LPARAM(large.0 as isize)),
                );
            },
            Err(error) => warn!("Official window icon unavailable: {error}"),
        }
        world.entity_mut(entity).insert(IconApplied);
    }
}
