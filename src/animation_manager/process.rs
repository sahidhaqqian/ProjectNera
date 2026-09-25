//! Keep the gallery's renderer/driver allocation lifetime separate from the pet.
//! Dropping Bevy assets doesn't release the renderer's high-water allocations.
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
};

use super::{
    AnimationManagerInputGate, AnimationManagerPlugin, ManagerWindow, OpenAnimationManager,
};
use bevy::{app::AppExit, prelude::*};

const PREFIX: &str = "NERA_MANAGER\t";

#[derive(Debug, PartialEq)]
enum Event {
    Play(String),
    Back,
}

fn decode_event(line: &str) -> Option<Event> {
    let message = line.strip_prefix(PREFIX)?;
    if message == "BACK" {
        return Some(Event::Back);
    }
    let path = message.strip_prefix("PLAY\t")?;
    let key = path.strip_prefix("animations/")?;
    if !key.to_ascii_lowercase().ends_with(".vrma")
        || key
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':'))
        || key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(Event::Play(path.to_owned()))
}

#[derive(Resource, Default)]
struct ManagerProcess {
    child: Option<Child>,
    events: Option<Mutex<mpsc::Receiver<Event>>>,
}

impl Drop for ManagerProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl ManagerProcess {
    fn spawn(&mut self, executable: &std::path::Path, smoke_test: bool) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(executable);
        command
            .arg("--animation-manager-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(0x0800_0000); // No console; the manager owns its own Bevy window.
        if smoke_test {
            command.env("NERA_GALLERY_SMOKE_TEST", "1");
        }
        let mut child = command.spawn()?;
        let output = child.stdout.take().expect("piped manager output");
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Some(event) = decode_event(&line) {
                    if send.send(event).is_err() {
                        break;
                    }
                }
            }
        });
        self.child = Some(child);
        self.events = Some(Mutex::new(receive));
        Ok(())
    }
}

pub(crate) struct AnimationManagerHostPlugin;
impl Plugin for AnimationManagerHostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ManagerProcess>()
            .init_resource::<AnimationManagerInputGate>()
            .add_message::<OpenAnimationManager>()
            .add_systems(Update, (poll_manager, open_manager).chain());
        #[cfg(debug_assertions)]
        if std::env::var_os("NERA_GALLERY_HOST_SMOKE_TEST").is_some() {
            app.add_systems(Update, host_smoke_test.after(open_manager));
        }
    }
}

fn open_manager(
    mut requests: MessageReader<OpenAnimationManager>,
    mut process: ResMut<ManagerProcess>,
) {
    if requests.read().next().is_none() {
        return;
    }
    if let Some(child) = &mut process.child {
        if let Some(input) = &mut child.stdin {
            let _ = writeln!(input, "FOCUS");
        }
        return;
    }
    let smoke =
        cfg!(debug_assertions) && std::env::var_os("NERA_GALLERY_HOST_SMOKE_TEST").is_some();
    let result = std::env::current_exe().and_then(|exe| process.spawn(&exe, smoke));
    if let Err(error) = result {
        error!("Could not open Animation Manager: {error}");
    }
}

fn poll_manager(
    mut process: ResMut<ManagerProcess>,
    mut play: MessageWriter<crate::animation::PlayLibraryAnimation>,
    mut back: MessageWriter<crate::settings::ReturnToActionMenu>,
) {
    if let Some(events) = &process.events {
        for event in events.lock().unwrap().try_iter() {
            match event {
                Event::Play(path) => {
                    play.write(crate::animation::PlayLibraryAnimation { path });
                }
                Event::Back => {
                    back.write(crate::settings::ReturnToActionMenu);
                }
            }
        }
    }
    if process
        .child
        .as_mut()
        .is_some_and(|child| child.try_wait().ok().flatten().is_some())
    {
        // The reader drains the pipe asynchronously; retain its receiver until
        // the next launch so the final Back/Play event cannot race process exit.
        process.child = None;
    }
}

#[derive(Resource)]
struct Controls(Mutex<mpsc::Receiver<()>>);

pub(crate) fn run_worker() {
    use bevy::render::{
        RenderPlugin,
        settings::{MemoryHints, WgpuSettings},
    };
    use bevy_vrm1::prelude::*;
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if line == "FOCUS" && send.send(()).is_err() {
                break;
            }
        }
        // EOF drops the sender. The worker exits if its parent terminates.
    });
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: crate::nera_asset_path(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    memory_hints: MemoryHints::MemoryUsage,
                    ..default()
                }
                .into(),
                ..default()
            }),
    );
    app.add_plugins((VrmPlugin, VrmaPlugin, AnimationManagerPlugin))
        .insert_resource(Controls(Mutex::new(receive)))
        .add_message::<crate::animation::PlayLibraryAnimation>()
        .add_message::<crate::settings::ReturnToActionMenu>()
        .add_systems(Startup, |mut open: MessageWriter<OpenAnimationManager>| {
            open.write_default();
        })
        .add_systems(
            Update,
            (
                receive_controls,
                forward_play.after(super::handle_actions),
                forward_back.after(super::close_after_release),
            ),
        );
    #[cfg(debug_assertions)]
    if std::env::var_os("NERA_GALLERY_SMOKE_TEST").is_some() {
        app.add_systems(Update, smoke_test.after(super::sync_gallery_scrollbar));
    }
    app.run();
}

fn receive_controls(
    controls: Res<Controls>,
    mut windows: Query<&mut Window, With<ManagerWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    loop {
        match controls.0.lock().unwrap().try_recv() {
            Ok(()) => {
                for mut window in &mut windows {
                    window.focused = true;
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                exit.write(AppExit::Success);
                break;
            }
            Err(mpsc::TryRecvError::Empty) => break,
        }
    }
}

fn send_event(message: &str) {
    let mut output = std::io::stdout().lock();
    let _ = writeln!(output, "{PREFIX}{message}");
    let _ = output.flush();
}
fn forward_play(mut play: MessageReader<crate::animation::PlayLibraryAnimation>) {
    for event in play.read() {
        send_event(&format!("PLAY\t{}", event.path));
    }
}
fn forward_back(mut back: MessageReader<crate::settings::ReturnToActionMenu>) {
    for _ in back.read() {
        send_event("BACK");
    }
}

#[cfg(debug_assertions)]
fn private_memory_bytes() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        cb: u32,
        faults: u32,
        peak_ws: usize,
        ws: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: isize, info: *mut Memory, size: u32) -> i32;
    }
    let mut memory = Memory::default();
    memory.cb = std::mem::size_of::<Memory>() as u32;
    assert_ne!(
        unsafe { GetProcessMemoryInfo(-1, &mut memory, std::mem::size_of::<Memory>() as u32) },
        0
    );
    memory.private
}

#[cfg(debug_assertions)]
#[derive(Default)]
struct HostSmoke {
    phase: u8,
    cycles: usize,
    baseline: usize,
    settle_at: f64,
}

#[cfg(debug_assertions)]
fn host_smoke_test(
    time: Res<Time>,
    process: Res<ManagerProcess>,
    mut state: Local<HostSmoke>,
    mut open: MessageWriter<OpenAnimationManager>,
    mut exit: MessageWriter<AppExit>,
) {
    match state.phase {
        0 if time.elapsed_secs_f64() > 8.0 => {
            state.baseline = private_memory_bytes();
            eprintln!("HOST_MEMORY baseline={} KiB", state.baseline / 1024);
            open.write_default();
            state.phase = 1;
        }
        1 if process.child.is_some() => state.phase = 2,
        2 if process.child.is_none() => {
            state.settle_at = time.elapsed_secs_f64() + 3.0;
            state.phase = 3;
        }
        3 if time.elapsed_secs_f64() >= state.settle_at => {
            let after = private_memory_bytes();
            state.cycles += 1;
            eprintln!(
                "HOST_MEMORY closed={} private={} KiB delta={} KiB",
                state.cycles,
                after / 1024,
                (after as i64 - state.baseline as i64) / 1024
            );
            assert!(
                after < state.baseline + 64 * 1024 * 1024,
                "desktop pet retained gallery memory"
            );
            if state.cycles == 2 {
                exit.write(AppExit::Success);
                state.phase = 4;
            } else {
                open.write_default();
                state.phase = 1;
            }
        }
        _ => {}
    }
}

#[cfg(debug_assertions)]
fn smoke_test(
    mut frame: Local<usize>,
    mut state: ResMut<super::ManagerState>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<super::GalleryScroll>>,
    slots: Query<&super::PreviewSlot>,
    mut requests: MessageWriter<crate::animation::PlayLibraryAnimation>,
) {
    *frame += 1;
    if *frame == 180 {
        let (mut position, computed) = scroll.single_mut().unwrap();
        position.0.y = super::scroll_metrics(computed).0;
        state.scroll_y = position.0.y;
    }
    if *frame == 300 {
        let count = state.filtered().len();
        assert!(slots.iter().any(|slot| slot.index == count - 1));
        requests.write(crate::animation::PlayLibraryAnimation {
            path: "animations/VRMA/Angry.vrma".into(),
        });
    }
    if *frame == 320 {
        state.close_requested = true;
        state.return_to_menu = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipe_protocol_only_accepts_library_paths_and_back() {
        assert_eq!(decode_event("NERA_MANAGER\tBACK"), Some(Event::Back));
        assert_eq!(
            decode_event("NERA_MANAGER\tPLAY\tanimations/VRMA/Angry.vrma"),
            Some(Event::Play("animations/VRMA/Angry.vrma".into()))
        );
        for line in [
            "a log line",
            "NERA_MANAGER\tPLAY\tC:/private.vrma",
            "NERA_MANAGER\tPLAY\tanimations/../private.vrma",
            "NERA_MANAGER\tPLAY\tanimations/a\n.vrma",
        ] {
            assert_eq!(decode_event(line), None);
        }
    }

    #[test]
    #[ignore = "opens and closes the built native gallery twice"]
    fn native_worker_exits_and_forwards_selection_and_back() {
        #[repr(C)]
        #[derive(Default)]
        struct Memory {
            cb: u32,
            faults: u32,
            peak_ws: usize,
            ws: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
            private: usize,
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(process: isize, info: *mut Memory, size: u32) -> i32;
        }
        fn parent_private_bytes() -> usize {
            let mut memory = Memory::default();
            memory.cb = std::mem::size_of::<Memory>() as u32;
            assert_ne!(
                unsafe {
                    GetProcessMemoryInfo(-1, &mut memory, std::mem::size_of::<Memory>() as u32)
                },
                0
            );
            memory.private
        }
        let baseline = parent_private_bytes();
        let executable =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/nera.exe");
        for _ in 0..2 {
            let mut process = ManagerProcess::default();
            process.spawn(&executable, true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
            let status = loop {
                if let Some(status) = process.child.as_mut().unwrap().try_wait().unwrap() {
                    break status;
                }
                assert!(std::time::Instant::now() < deadline, "worker did not close");
                std::thread::sleep(std::time::Duration::from_millis(100));
            };
            assert!(status.success());
            let events: Vec<_> = process
                .events
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .iter()
                .collect();
            assert!(events.contains(&Event::Play("animations/VRMA/Angry.vrma".into())));
            assert!(events.contains(&Event::Back));
            let after = parent_private_bytes();
            eprintln!(
                "Gallery worker exited successfully; parent private memory: baseline={} KiB after={} KiB",
                baseline / 1024,
                after / 1024
            );
            assert!(
                after < baseline + 32 * 1024 * 1024,
                "gallery memory must not remain in the parent process"
            );
        }
    }
}
