//! Bounded, opt-in Windows audit; no instrumentation in normal launches.
use super::*;
use bevy::{text::FontAtlasSet, window::WindowCloseRequested};

#[derive(Resource, Default)]
struct Audit {
    phase: u8,
    cycle: usize,
    next: f64,
    screenshots: bool,
    framing_only: bool,
    imported_sound: Option<u16>,
    ring_source: Option<AssetId<bevy::audio::AudioSource>>,
}

pub(super) fn install(app: &mut App) {
    let framing_only = std::env::args().any(|arg| arg == "--alarm-framing-audit");
    app.insert_resource(Audit {
        screenshots: framing_only
            || std::env::args().any(|arg| arg == "--alarm-memory-screenshots"),
        framing_only,
        cycle: if framing_only { 5 } else { 0 },
        ..default()
    })
    .add_systems(Last, advance);
}

fn snapshot(world: &mut World, stage: &str) {
    let (private, working, _) = crate::animation_manager::process::memory_usage();
    let windows = world.query::<&AlarmSettingsWindow>().iter(world).count();
    let owned = world.query::<&AlarmSettingsOwned>().iter(world).count();
    let (dismiss_windows, dismiss_owned) = crate::alarm_dismiss::memory_audit_counts(world);
    let (ring_players, preview_players, audio_bytes) = crate::alarm::memory_audit_counts(world);
    let cameras = world.query::<&Camera>().iter(world).count();
    let nodes = world.query::<&Node>().iter(world).count();
    let main_size = world
        .query_filtered::<&Window, With<bevy::window::PrimaryWindow>>()
        .single(world)
        .map(|window| [window.physical_width(), window.physical_height()])
        .unwrap();
    let attention = world
        .resource::<crate::presentation::PresentationState>()
        .is_attention();
    let fonts = world.resource::<FontAtlasSet>();
    let images = world.resource::<Assets<Image>>();
    let record = serde_json::json!({
        "stage": stage, "private_mib": private as f64 / 1048576.0,
        "working_set_mib": working as f64 / 1048576.0,
        "editor_windows": windows, "editor_owned": owned,
        "dismiss_windows": dismiss_windows, "dismiss_owned": dismiss_owned,
        "cameras": cameras, "ui_nodes": nodes,
        "main_window_physical": main_size, "attention": attention,
        "font_keys": fonts.len(), "font_textures": fonts.values().map(Vec::len).sum::<usize>(),
        "font_cpu_mib": fonts.total_bytes(images) as f64 / 1048576.0,
        "images": images.len(), "ring_players": ring_players, "preview_players": preview_players,
        "audio_cpu_mib": audio_bytes as f64 / 1048576.0,
        "animation_clips": world.resource::<Assets<AnimationClip>>().len(),
    });
    eprintln!("NERA_ALARM_MEMORY {record}");
}

fn screenshot(world: &mut World, title: &str, name: &str) {
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    let window = world
        .query::<(Entity, &Window)>()
        .iter(world)
        .find_map(|(entity, window)| (window.title == title).then_some(entity))
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(std::env::temp_dir().join(name)));
}

fn action(world: &mut World, action: AlarmSettingsAction) {
    let button = world
        .spawn((Button, action, Interaction::Pressed, BackgroundColor(BLUE)))
        .id();
    world
        .run_system_cached(handle_alarm_settings_actions)
        .unwrap();
    world.despawn(button);
}

fn capture_portrait(world: &mut World, seconds: u8) {
    screenshot(
        world,
        "Nera",
        &format!("nera-alarm-portrait-{seconds}s.png"),
    );
    let pose = world
        .resource::<crate::animation::NeraAnimationState>()
        .visual_state();
    let bones: Vec<_> = world
        .query_filtered::<(&bevy_vrm1::prelude::VrmBone, &GlobalTransform), With<crate::interaction::NeraLiveBone>>()
        .iter(world)
        .filter(|(bone, _)| matches!(bone.as_str(), "head" | "hips"))
        .map(|(bone, transform)| (bone.as_str().to_owned(), transform.translation()))
        .collect();
    eprintln!("NERA_ALARM_PORTRAIT seconds={seconds} pose={pose:?} bones={bones:?}");
}

fn summon(world: &mut World, cycle: usize) {
    if cycle < 3 {
        world.write_message(OpenAlarmSettingsWindow);
    } else {
        world.write_message(crate::action_menu::NeraActionMenuCommand::Alarm(
            crate::action_menu::AlarmMenuChoice::TestNow,
        ));
    }
}

fn advance(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    if now < 10.0 || now < world.resource::<Audit>().next {
        return;
    }
    assert!(now < 180.0, "alarm memory audit timed out");
    world.resource_scope(|world, mut audit: Mut<Audit>| {
        match audit.phase {
            0 => {
                world.resource_mut::<NeraSettings>().audio_muted = true;
                snapshot(world, "before_open");
                summon(world, audit.cycle);
                audit.phase = 1;
                audit.next = now + 3.0;
            }
            1 => {
                snapshot(world, &format!("open_{}", audit.cycle + 1));
                if audit.screenshots && audit.cycle >= 3 {
                    capture_portrait(world, 3);
                }
                if audit.framing_only {
                    audit.phase = 6;
                    audit.next = now + 6.0;
                    return;
                }
                if audit.screenshots && audit.cycle == 2 {
                    screenshot(world, "Nera Alarms", "nera-alarm-memory-list.png");
                }
                if audit.screenshots && audit.cycle == 3 {
                    screenshot(world, "Nera Alarm", "nera-alarm-memory-dismiss.png");
                }
                if audit.cycle < 3 {
                    action(world, AlarmSettingsAction::Add);
                    audit.phase = 2;
                    audit.next = now + 1.0;
                } else {
                    if audit.cycle == 4 {
                        let window = world
                            .query::<(Entity, &crate::alarm_dismiss::AlarmDismissWindow)>()
                            .iter(world)
                            .next()
                            .unwrap()
                            .0;
                        world.write_message(WindowCloseRequested { window });
                    } else {
                        world
                            .write_message(crate::presentation::PresentationCommand::ExitAttention);
                    }
                    audit.phase = 5;
                    audit.next = now + 6.0;
                }
            }
            2 => {
                action(world, AlarmSettingsAction::OpenTimePicker);
                audit.phase = 3;
                audit.next = now + 1.0;
            }
            3 => {
                action(world, AlarmSettingsAction::Back);
                action(world, AlarmSettingsAction::OpenDatePicker);
                audit.phase = 4;
                audit.next = now + 1.0;
            }
            4 => {
                snapshot(world, &format!("picker_{}", audit.cycle + 1));
                if audit.screenshots && audit.cycle == 2 {
                    screenshot(world, "Nera Alarms", "nera-alarm-memory-date.png");
                }
                if audit.cycle == 0 {
                    // This audit owns an isolated catalog; never import/delete
                    // from a user's real alarm sound library.
                    let directory = std::env::temp_dir().join(format!("nera-alarm-sound-audit-{}", std::process::id()));
                    std::fs::create_dir_all(&directory).unwrap();
                    let original = crate::nera_asset_file("sound/clock_beeps.wav");
                    let renamed = directory.join("alarm-copy.wav");
                    std::fs::copy(&original, &renamed).unwrap();
                    let mut library = AlarmSoundLibrary::load(directory.join("library"));
                    let (id, _) = library.import(&original).unwrap();
                    assert_eq!(library.import(&renamed).unwrap(), (id, true));
                    assert_eq!(library.entries().len(), 1);
                    assert!(!library.remove(crate::alarm::sounds::DEFAULT_SOUND).unwrap());
                    world.insert_resource(library);
                    audit.imported_sound = Some(id);
                    action(world, AlarmSettingsAction::SelectSound(id));
                    action(world, AlarmSettingsAction::Save);
                    assert!(world.resource::<NeraSettings>().alarm_configs().iter().any(|alarm| alarm.sound == id));
                    action(world, AlarmSettingsAction::OpenSoundLibrary);
                    action(world, AlarmSettingsAction::PreviewSound(id));
                    audit.phase = 20;
                    audit.next = now + 1.0;
                    return;
                }
                // Close while a long sound preview is still playing/loading.
                world.write_message(crate::alarm::sounds::PreviewAlarmSound(0));
                if audit.cycle == 1 {
                    let window = world
                        .query::<(Entity, &AlarmSettingsWindow)>()
                        .iter(world)
                        .next()
                        .unwrap()
                        .0;
                    world.write_message(WindowCloseRequested { window });
                } else {
                    action(world, AlarmSettingsAction::Close);
                }
                audit.phase = 5;
                audit.next = now + 6.0;
            }
            5 => {
                snapshot(world, &format!("closed_{}", audit.cycle + 1));
                assert_eq!(world.query::<&AlarmSettingsOwned>().iter(world).count(), 0);
                assert_eq!(world.query::<&AlarmSettingsWindow>().iter(world).count(), 0);
                assert_eq!(crate::alarm_dismiss::memory_audit_counts(world), (0, 0));
                assert_eq!(crate::alarm::memory_audit_counts(world).0, 0);
                assert_eq!(crate::alarm::memory_audit_counts(world).1, 0);
                assert_eq!(
                    world
                        .query::<&crate::utility_window::font_cache::FontCacheOwner>()
                        .iter(world)
                        .count(),
                    0
                );
                assert!(world.resource::<FontAtlasSet>().is_empty());
                audit.cycle += 1;
                if audit.cycle == 6 {
                    if audit.framing_only {
                        eprintln!("ALARM_FRAMING_AUDIT_PASS captures=3");
                    } else {
                        eprintln!("ALARM_MEMORY_AUDIT_PASS editor_cycles=3 ring_cycles=3");
                    }
                    world.write_message(AppExit::Success);
                    audit.next = f64::INFINITY;
                } else {
                    summon(world, audit.cycle);
                    audit.phase = 1;
                    audit.next = now + 3.0;
                }
            }
            20 => {
                assert_eq!(crate::alarm::memory_audit_counts(world).1, 1);
                if audit.screenshots { screenshot(world, "Nera Alarms", "nera-alarm-sound-library.png"); }
                action(world, AlarmSettingsAction::PreviewSound(0));
                audit.phase = 21;
                audit.next = now + 0.5;
            }
            21 => {
                assert_eq!(crate::alarm::memory_audit_counts(world).1, 1);
                action(world, AlarmSettingsAction::StopPreview);
                audit.phase = 22;
                audit.next = now + 0.5;
            }
            22 => {
                assert_eq!(crate::alarm::memory_audit_counts(world).1, 0);
                let id = audit.imported_sound.unwrap();
                action(world, AlarmSettingsAction::DeleteSound(id));
                assert!(world.resource::<NeraSettings>().alarm_configs().iter().all(|alarm| alarm.sound != id));
                assert!(world.resource::<AlarmSoundLibrary>().entries().is_empty());
                action(world, AlarmSettingsAction::SelectSound(crate::alarm::sounds::RANDOM_SOUND));
                world.write_message(NeraActionMenuCommand::Alarm(AlarmMenuChoice::TestNow));
                audit.phase = 23;
                audit.next = now + 3.0;
            }
            23 => {
                assert_eq!(crate::alarm::memory_audit_counts(world).0, 1);
                audit.ring_source = crate::alarm::memory_audit_ring_source(world);
                assert!(audit.ring_source.is_some());
                action(world, AlarmSettingsAction::PreviewSound(0));
                audit.phase = 24;
                audit.next = now + 1.0;
            }
            24 => {
                let (ring, preview, _) = crate::alarm::memory_audit_counts(world);
                assert_eq!((ring, preview), (1, 1));
                assert_eq!(crate::alarm::memory_audit_ring_source(world), audit.ring_source);
                action(world, AlarmSettingsAction::Close);
                audit.phase = 25;
                audit.next = now + 1.0;
            }
            25 => {
                let (ring, preview, _) = crate::alarm::memory_audit_counts(world);
                assert_eq!((ring, preview), (1, 0));
                assert_eq!(crate::alarm::memory_audit_ring_source(world), audit.ring_source);
                eprintln!("ALARM_SOUND_LIBRARY_AUDIT_PASS import_dedup_select_delete_random_preview_close_ring_isolation=true");
                world.write_message(crate::presentation::PresentationCommand::ExitAttention);
                audit.phase = 5;
                audit.next = now + 6.0;
            }
            6 => {
                capture_portrait(world, 9);
                audit.phase = 7;
                audit.next = now + 20.0;
            }
            7 => {
                capture_portrait(world, 29);
                // Let the screenshot render before the real dismissal restores the camera.
                audit.phase = 8;
                audit.next = now + 1.0;
            }
            8 => {
                world.write_message(crate::presentation::PresentationCommand::ExitAttention);
                audit.phase = 5;
                audit.next = now + 6.0;
            }
            _ => unreachable!(),
        }
    });
}
