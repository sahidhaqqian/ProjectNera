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
    pending_playback_error: Option<(String, String)>,
    character_path: String,
}

impl Drop for ManagerProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            crate::platform::set_animation_manager_process_id(None);
        }
    }
}

impl ManagerProcess {
    fn spawn(
        &mut self,
        executable: &std::path::Path,
        smoke_test: bool,
        character_path: &str,
    ) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(executable);
        command
            .arg("--animation-manager-worker")
            .env("NERA_GALLERY_CHARACTER", character_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(0x0800_0000); // No console; the manager owns its own Bevy window.
        if smoke_test {
            command.env("NERA_GALLERY_SMOKE_TEST", "1");
        }
        let mut child = command.spawn()?;
        crate::platform::set_animation_manager_process_id(Some(child.id()));
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
        self.character_path = character_path.to_owned();
        Ok(())
    }
}

pub(crate) struct AnimationManagerHostPlugin;
impl Plugin for AnimationManagerHostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ManagerProcess>()
            .init_resource::<AnimationManagerInputGate>()
            .add_message::<OpenAnimationManager>()
            .add_systems(
                Update,
                (
                    poll_manager,
                    open_manager,
                    forward_character,
                    forward_playback_errors,
                )
                    .chain()
                    .before(crate::pet_behavior::update_pet_behavior),
            );
        #[cfg(debug_assertions)]
        if std::env::var_os("NERA_GALLERY_HOST_SMOKE_TEST").is_some() {
            app.add_systems(Update, host_smoke_test.after(open_manager));
        }
    }
}

fn open_manager(
    mut requests: MessageReader<OpenAnimationManager>,
    mut process: ResMut<ManagerProcess>,
    mut gate: ResMut<AnimationManagerInputGate>,
    character: Option<Res<crate::character::ActiveCharacter>>,
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
    let path = character.as_ref().map_or(
        crate::character::DEFAULT_CHARACTER,
        |character| character.asset_path.as_str(),
    );
    let result = std::env::current_exe().and_then(|exe| process.spawn(&exe, smoke, path));
    if let Err(error) = result {
        error!("Could not open Animation Manager: {error}");
    }
    gate.window_open = process.child.is_some();
}

fn forward_character(
    character: Option<Res<crate::character::ActiveCharacter>>,
    mut process: ResMut<ManagerProcess>,
) {
    let Some(character) = character else {
        return;
    };
    if process.child.is_none() || process.character_path == character.asset_path {
        return;
    }
    // Only ActiveCharacter is forwarded: a failed import keeps the last rig in
    // both processes. JSON keeps spaces, quotes and Unicode file names intact.
    let encoded = serde_json::to_string(&character.asset_path).expect("serializing asset path");
    if process
        .child
        .as_mut()
        .and_then(|child| child.stdin.as_mut())
        .is_some_and(|input| writeln!(input, "CHARACTER\t{encoded}").is_ok())
    {
        process.character_path.clone_from(&character.asset_path);
    }
}

fn poll_manager(
    mut process: ResMut<ManagerProcess>,
    mut gate: ResMut<AnimationManagerInputGate>,
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
        crate::platform::set_animation_manager_process_id(None);
    }
    gate.window_open = process.child.is_some();
}

fn forward_playback_errors(
    mut errors: MessageReader<crate::animation::LibraryAnimationError>,
    mut process: ResMut<ManagerProcess>,
) {
    for error in errors.read() {
        process.pending_playback_error = Some((error.path.clone(), error.reason.clone()));
    }
    let Some(error) = &process.pending_playback_error else {
        return;
    };
    // Keep the latest error until a gallery is available, including load failures
    // arriving after the user closed it. JSON preserves file names and newlines.
    let encoded = serde_json::to_string(error).expect("serializing two strings");
    if process
        .child
        .as_mut()
        .and_then(|child| child.stdin.as_mut())
        .is_some_and(|input| writeln!(input, "PLAYBACK_ERROR\t{encoded}").is_ok())
    {
        process.pending_playback_error = None;
    }
}

#[derive(Debug, PartialEq)]
enum Control {
    Focus,
    PlaybackError(String, String),
    Character(String),
}

fn decode_control(line: &str) -> Option<Control> {
    if line == "FOCUS" {
        return Some(Control::Focus);
    }
    if let Some(path) = line.strip_prefix("CHARACTER\t") {
        return serde_json::from_str::<String>(path)
            .ok()
            .map(Control::Character);
    }
    let (path, reason) =
        serde_json::from_str::<(String, String)>(line.strip_prefix("PLAYBACK_ERROR\t")?).ok()?;
    Some(Control::PlaybackError(path, reason))
}

#[derive(Resource)]
struct Controls(Mutex<mpsc::Receiver<Control>>);

pub(crate) fn run_worker() {
    use bevy::render::{
        RenderPlugin,
        settings::{MemoryHints, WgpuSettings},
    };
    use bevy_vrm1::prelude::*;
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if let Some(control) = decode_control(&line) {
                if send.send(control).is_err() {
                    break;
                }
            }
        }
        // EOF drops the sender. The worker exits if its parent terminates.
    });
    let mut app = App::new();
    crate::character::register_asset_source(&mut app);
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: crate::nera_asset_path(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                close_when_requested: false,
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
    let character_path = std::env::var("NERA_GALLERY_CHARACTER")
        .unwrap_or_else(|_| crate::character::DEFAULT_CHARACTER.to_owned());
    app.add_plugins((
        VrmPlugin,
        VrmaPlugin,
        crate::utility_window::UtilityWindowPlugin,
        AnimationManagerPlugin,
    ))
    .insert_resource(super::preview::PreviewRuntime::for_character(character_path))
    .insert_resource(Controls(Mutex::new(receive)))
    .add_message::<crate::animation::PlayLibraryAnimation>()
    .add_message::<crate::settings::ReturnToActionMenu>()
    .add_systems(Startup, |mut open: MessageWriter<OpenAnimationManager>| {
        open.write_default();
    })
    .add_systems(
        Update,
        (
            receive_controls.before(super::preview::sync_visible_previews),
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
    mut state: ResMut<super::ManagerState>,
    mut exit: MessageWriter<AppExit>,
    mut previews: ResMut<super::preview::PreviewRuntime>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    loop {
        match controls.0.lock().unwrap().try_recv() {
            Ok(Control::Focus) => {
                for mut window in &mut windows {
                    window.set_minimized(false);
                    window.focused = true;
                }
            }
            Ok(Control::PlaybackError(path, reason)) => {
                let name = path.rsplit('/').next().unwrap_or(&path);
                state.error = Some(format!("{name}: {reason}"));
                state.dirty = true;
            }
            Ok(Control::Character(path)) => {
                previews.set_character(path, &mut commands, &mut images);
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
    fn character_protocol_preserves_asset_paths_and_rejects_malformed_messages() {
        let path = "characters://Nera \"custom\" 日本語.vrm";
        assert_eq!(
            decode_control(&format!("CHARACTER\t{}", serde_json::to_string(path).unwrap())),
            Some(Control::Character(path.into()))
        );
        assert_eq!(decode_control("CHARACTER\tmalformed"), None);
        assert_eq!(decode_control("CHARACTER\t42"), None);
    }

    #[test]
    fn desktop_playback_failure_reaches_the_gallery_error_label() {
        let path = "animations/imported/quoted \"name\".vrma";
        let reason = "Invalid VRMA\nSelect another animation";
        let line = format!(
            "PLAYBACK_ERROR\t{}",
            serde_json::to_string(&(path, reason)).unwrap()
        );
        assert_eq!(
            decode_control(&line),
            Some(Control::PlaybackError(path.into(), reason.into()))
        );
        assert_eq!(decode_control("FOCUS"), Some(Control::Focus));
        assert_eq!(decode_control("PLAYBACK_ERROR\tmalformed"), None);
        let (send, receive) = mpsc::channel();
        send.send(decode_control(&line).unwrap()).unwrap();
        let mut app = App::new();
        let mut state = super::super::ManagerState::for_test();
        state.dirty = false;
        app.insert_resource(state)
            .insert_resource(Controls(Mutex::new(receive)))
            .init_resource::<super::super::preview::PreviewRuntime>()
            .init_resource::<Assets<Image>>()
            .add_message::<AppExit>()
            .add_systems(Update, receive_controls);
        app.update();
        let state = app.world().resource::<super::super::ManagerState>();
        assert!(state.dirty);
        assert_eq!(
            state.error.as_deref(),
            Some("quoted \"name\".vrma: Invalid VRMA\nSelect another animation")
        );
    }

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
            process.spawn(&executable, true, crate::character::DEFAULT_CHARACTER).unwrap();
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
