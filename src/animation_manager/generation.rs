//! One disposable renderer for a full-duration character idle cache.
//! Process exit releases decoder, renderer and allocator high-water marks as well as ECS assets.
use bevy::prelude::*;

#[derive(Resource)]
pub(super) struct ThumbnailGeneration;

pub(super) fn install(app: &mut App, path: String) {
    app.insert_resource(ThumbnailGeneration)
        .add_systems(Startup, move |mut state: ResMut<super::ManagerState>| {
            state.tab = super::GalleryTab::Characters;
            state.characters.models = vec![crate::character::library::CharacterEntry {
                path: path.clone(),
                name: "Preview".into(),
                ..Default::default()
            }];
            state.characters.slots.clear();
            state.dirty = true;
        })
        .add_systems(
            Update,
            hide_window.before(super::preview::sync_visible_previews),
        )
        .add_systems(Last, finish);
}

fn hide_window(mut windows: Query<&mut Window, With<super::ManagerWindow>>) {
    for mut window in &mut windows {
        window.visible = false;
    }
}

fn finish(
    runtime: Res<super::preview::PreviewRuntime>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
    mut deadline: Local<Option<f64>>,
) {
    if runtime.generation_complete() {
        exit.write(AppExit::Success);
    } else if time.elapsed_secs_f64()
        > *deadline.get_or_insert_with(|| super::idle_cache::required_seconds() * 2.0 + 30.0)
    {
        exit.write(AppExit::error());
    }
}
