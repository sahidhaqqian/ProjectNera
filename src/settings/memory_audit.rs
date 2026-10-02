//! Opt-in native Settings open/close audit; never runs during ordinary launches.
use super::ui::{SettingsButtonAction, SettingsOwned, SettingsWindow};
use bevy::{prelude::*, text::FontAtlasSet, window::WindowCloseRequested};

#[derive(Resource, Default)]
struct Audit {
    phase: u8,
    cycle: usize,
    next: f64,
    baseline: usize,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Audit>().add_systems(Last, advance);
}

fn snapshot(world: &mut World, stage: &str) -> usize {
    let (private, working, peak) = crate::animation_manager::process::memory_usage();
    let owned = world.query::<&SettingsOwned>().iter(world).count();
    let windows = world
        .query_filtered::<Entity, With<SettingsWindow>>()
        .iter(world)
        .count();
    let entities = world.query::<Entity>().iter(world).count();
    let nodes = world.query::<&Node>().iter(world).count();
    let cameras = world.query::<&Camera>().iter(world).count();
    let glyphs = world
        .query::<&bevy::text::TextLayoutInfo>()
        .iter(world)
        .map(|layout| layout.glyphs.len())
        .sum::<usize>();
    let images = world.resource::<Assets<Image>>();
    let fonts = world.resource::<FontAtlasSet>();
    let record = serde_json::json!({
        "stage": stage, "pid": std::process::id(),
        "private_mib": private as f64 / 1048576.0, "working_set_mib": working as f64 / 1048576.0,
        "peak_private_mib": peak as f64 / 1048576.0,
        "settings_windows": windows, "settings_owned": owned, "entities": entities,
        "ui_nodes": nodes, "cameras": cameras, "glyphs": glyphs,
        "font_keys": fonts.len(), "font_textures": fonts.values().map(Vec::len).sum::<usize>(),
        "font_cpu_mib": fonts.total_bytes(images) as f64 / 1048576.0,
        "images": images.len(), "cpu_pixels_mib": images.iter().map(|(_, image)| image.data.as_ref().map_or(0, Vec::len)).sum::<usize>() as f64 / 1048576.0,
        "animation_clips": world.resource::<Assets<AnimationClip>>().len(),
    });
    eprintln!("NERA_SETTINGS_MEMORY {record}");
    private
}

fn advance(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    if now < 10.0 || now < world.resource::<Audit>().next {
        return;
    }
    assert!(now < 120.0, "Settings memory audit timed out");
    world.resource_scope(|world, mut audit: Mut<Audit>| {
        match audit.phase {
            0 => {
                audit.baseline = snapshot(world, "before_open");
                world.write_message(super::OpenSettingsWindow);
                audit.phase = 1;
                audit.next = now + 4.0;
            }
            1 => {
                snapshot(world, &format!("open_{}", audit.cycle + 1));
                let window = world
                    .query_filtered::<Entity, With<SettingsWindow>>()
                    .single(world)
                    .unwrap();
                if audit.cycle == 3 {
                    // Use the real OS close-request path, not a late direct despawn.
                    world.write_message(WindowCloseRequested { window });
                } else {
                    let button = world
                        .query::<(Entity, &SettingsButtonAction)>()
                        .iter(world)
                        .find_map(|(entity, action)| {
                            matches!(action, SettingsButtonAction::CloseWindow).then_some(entity)
                        })
                        .unwrap();
                    *world.get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
                    world
                        .run_system_cached(super::ui::handle_settings_buttons)
                        .unwrap();
                }
                audit.phase = 2;
                audit.next = now + 6.0;
            }
            2 => {
                let closed = snapshot(world, &format!("closed_{}", audit.cycle + 1));
                assert_eq!(world.query::<&SettingsOwned>().iter(world).count(), 0);
                assert_eq!(
                    world
                        .query_filtered::<Entity, With<SettingsWindow>>()
                        .iter(world)
                        .count(),
                    0
                );
                eprintln!(
                    "SETTINGS_MEMORY_DELTA cycle={} private_mib={:.2}",
                    audit.cycle + 1,
                    (closed as f64 - audit.baseline as f64) / 1048576.0
                );
                audit.cycle += 1;
                if audit.cycle == 6 {
                    eprintln!("SETTINGS_MEMORY_AUDIT_PASS cycles=6");
                    world.write_message(AppExit::Success);
                    audit.next = f64::INFINITY;
                } else {
                    world.write_message(super::OpenSettingsWindow);
                    audit.phase = 1;
                    audit.next = now + 4.0;
                }
            }
            _ => unreachable!(),
        }
    });
}
