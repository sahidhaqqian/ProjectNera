//! Settings joins the shared utility-window font-cache lifecycle before opening.
use super::ui::{OpenSettingsWindow, SettingsWindow};
pub(super) use crate::utility_window::font_cache::UtilityFontCache as SettingsFontCache;
#[cfg(test)]
use crate::utility_window::font_cache::{FontCacheOwner, release};
use bevy::{prelude::*, text::FontAtlasSet};

pub(super) fn begin(
    mut open: MessageReader<OpenSettingsWindow>,
    windows: Query<(), With<SettingsWindow>>,
    fonts: Option<Res<FontAtlasSet>>,
    mut cache: ResMut<SettingsFontCache>,
) {
    if open.read().next().is_none() || !windows.is_empty() {
        return;
    }
    cache.begin(fonts.as_deref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        asset::AssetId,
        text::{FontAtlas, FontAtlasKey, GlyphAtlasInfo, PositionedGlyph, TextLayoutInfo},
    };

    fn key(id: u32) -> FontAtlasKey {
        FontAtlasKey {
            id,
            index: 0,
            font_size_bits: 14.0f32.to_bits(),
            variations_hash: 0,
            hinting: default(),
            font_smoothing: default(),
        }
    }
    fn atlas(app: &mut App, id: u32) -> AssetId<Image> {
        let atlas = FontAtlas::new(
            &mut app.world_mut().resource_mut::<Assets<Image>>(),
            UVec2::splat(16),
            default(),
        );
        let image = atlas.texture.id();
        app.world_mut()
            .resource_mut::<FontAtlasSet>()
            .insert(key(id), vec![atlas]);
        image
    }
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<FontAtlasSet>()
            .init_resource::<SettingsFontCache>()
            .add_message::<OpenSettingsWindow>()
            .add_systems(Update, begin)
            .add_systems(Last, release);
        app
    }
    fn enter(app: &mut App) -> Entity {
        // begin precedes actual window creation in the production Update chain.
        app.world_mut().write_message(OpenSettingsWindow);
        app.world_mut().run_system_cached(begin).unwrap();
        app.world_mut().spawn((SettingsWindow, FontCacheOwner)).id()
    }
    #[test]
    fn closing_reclaims_only_settings_fonts_and_repeated_cycles_are_bounded() {
        let mut app = app();
        let persistent = atlas(&mut app, 1);
        let avatar = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        for _ in 0..6 {
            let window = enter(&mut app);
            let temporary = atlas(&mut app, 2);
            app.update();
            assert!(app.world().resource::<Assets<Image>>().contains(temporary));
            app.world_mut().despawn(window);
            app.update();
            assert!(!app.world().resource::<Assets<Image>>().contains(temporary));
            assert_eq!(app.world().resource::<FontAtlasSet>().len(), 1);
            assert!(app.world().resource::<Assets<Image>>().contains(persistent));
            assert!(
                app.world()
                    .resource::<Assets<Image>>()
                    .contains(avatar.id())
            );
            assert!(!app.world().resource::<SettingsFontCache>().is_active());
        }
    }
    #[test]
    fn shared_glyphs_and_images_survive_settings_close_even_when_hidden() {
        let mut app = app();
        let window = enter(&mut app);
        let shared = atlas(&mut app, 2);
        let image = atlas(&mut app, 3);
        let unused = atlas(&mut app, 4);
        app.world_mut().spawn((
            Visibility::Hidden,
            TextLayoutInfo {
                glyphs: vec![PositionedGlyph {
                    position: Vec2::ZERO,
                    section_index: 0,
                    line_index: 0,
                    atlas_info: GlyphAtlasInfo {
                        texture: shared,
                        rect: default(),
                        offset: Vec2::ZERO,
                        is_alpha_mask: true,
                    },
                }],
                ..default()
            },
        ));
        let handle = app.world().resource::<FontAtlasSet>()[&key(3)][0]
            .texture
            .clone();
        app.world_mut().spawn(ImageNode::new(handle));
        app.world_mut().despawn(window);
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        assert!(images.contains(shared) && images.contains(image));
        assert!(!images.contains(unused));
        assert_eq!(app.world().resource::<FontAtlasSet>().len(), 2);
    }
    #[test]
    fn duplicate_open_does_not_reclassify_temporary_fonts_as_persistent() {
        let mut app = app();
        let window = enter(&mut app);
        let temporary = atlas(&mut app, 2);
        app.world_mut().write_message(OpenSettingsWindow);
        app.update();
        app.world_mut().despawn(window);
        app.update();
        assert!(!app.world().resource::<Assets<Image>>().contains(temporary));
    }

    #[test]
    fn overlapping_utility_windows_release_shared_fonts_only_after_last_owner_closes() {
        let mut app = app();
        let settings = enter(&mut app);
        let first_font = atlas(&mut app, 1);
        // Another utility window must join, not reclassify Settings fonts as baseline.
        app.world_mut()
            .resource_mut::<SettingsFontCache>()
            .begin(None);
        let alarm = app.world_mut().spawn(FontCacheOwner).id();
        let alarm_camera = app.world_mut().spawn(FontCacheOwner).id();
        let second_font = atlas(&mut app, 2);
        app.world_mut().despawn(settings);
        app.update();
        assert!(app.world().resource::<Assets<Image>>().contains(first_font));
        app.world_mut().despawn(alarm);
        app.update();
        assert!(app.world().resource::<SettingsFontCache>().is_active());
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .contains(second_font)
        );
        app.world_mut().despawn(alarm_camera);
        app.update();
        assert!(app.world().resource::<FontAtlasSet>().is_empty());
        assert!(!app.world().resource::<Assets<Image>>().contains(first_font));
        assert!(
            !app.world()
                .resource::<Assets<Image>>()
                .contains(second_font)
        );
    }

    #[test]
    fn os_close_waits_for_owned_text_cleanup_before_releasing_font_cache() {
        let mut app = app();
        app.init_resource::<super::super::NeraSettings>()
            .init_resource::<super::super::runtime::StartupRuntime>()
            .init_resource::<super::super::ui::SettingsInputGate>();
        app.world_mut().write_message(OpenSettingsWindow);
        app.world_mut().run_system_cached(begin).unwrap();
        app.world_mut()
            .run_system_cached(super::super::ui::open_settings_window)
            .unwrap();
        let temporary = atlas(&mut app, 2);
        let world = app.world_mut();
        let window = world
            .query_filtered::<Entity, With<SettingsWindow>>()
            .single(world)
            .unwrap();
        world.despawn(window);
        app.update();
        assert!(app.world().resource::<SettingsFontCache>().is_active());
        assert!(app.world().resource::<Assets<Image>>().contains(temporary));
        app.world_mut()
            .run_system_cached(super::super::ui::cleanup_settings_window)
            .unwrap();
        app.update();
        assert!(!app.world().resource::<SettingsFontCache>().is_active());
        assert!(app.world().resource::<FontAtlasSet>().is_empty());
        assert!(!app.world().resource::<Assets<Image>>().contains(temporary));
    }
}
