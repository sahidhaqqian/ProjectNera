//! Avatar scene construction and the runtime animation asset set.
use super::{LookAtTarget, NeraAvatar};
use crate::animation::{
    register_angry_animation, register_death_animation, register_falling_animation,
    register_falling_to_landing_animation, register_floating_animation,
    register_head_stroke_animation, register_hit_chest_animation, register_hit_head_animation,
    register_idle_animation, register_idle_blush_animation, register_idle_look_around_animation,
    register_idle_relax_animation, register_idle_sleepy_animation, register_praying_animation,
    register_sad_animation, register_sitting_enter_animation, register_sitting_exit_animation,
    register_sitting_loop_animation, register_surprised_animation, register_thinking_animation,
    register_wave_animation,
};
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

const VRM_PATH: &str = "vrm/nera_256.vrm";

// Phase 5 runtime animation set. Large animation libraries stay as stock;
// only stable animations used by current behavior live in animasi_basic.
const IDLE_VRMA_PATH: &str = "animations/animasi_basic/idle.vrma";
const FLOATING_VRMA_PATH: &str = "animations/animasi_basic/Floating.vrma";
const DEATH_VRMA_PATH: &str = "animations/animasi_basic/Death01.vrma";
const FALLING_VRMA_PATH: &str = "animations/Falling.vrma";
const FALLING_TO_LANDING_VRMA_PATH: &str = "animations/Falling_To_Landing.vrma";
const HIT_HEAD_VRMA_PATH: &str = "animations/animasi_basic/Hit_Head.vrma";
const HIT_CHEST_VRMA_PATH: &str = "animations/animasi_basic/Hit_Chest.vrma";
const HEAD_STROKE_VRMA_PATH: &str = "animations/animasi_basic/ShakeHeadNo1.vrma";
const PRAYING_VRMA_PATH: &str = "animations/animasi_basic/Praying.vrma";
const SITTING_ENTER_VRMA_PATH: &str = "animations/animasi_basic/Sitting_Enter.vrma";
const SITTING_LOOP_VRMA_PATH: &str = "animations/animasi_basic/Sitting_Idle_Loop.vrma";
const SITTING_EXIT_VRMA_PATH: &str = "animations/animasi_basic/Sitting_Exit.vrma";

// Phase 5.8 idle micro-actions. Fidgets only: Clapping/Goodbye/Jump are also
// converted and unused, but a pet that randomly applauds while you work reads
// as noise rather than life.
const IDLE_LOOK_AROUND_VRMA_PATH: &str = "animations/VRMA/LookAround.vrma";
const IDLE_RELAX_VRMA_PATH: &str = "animations/VRMA/Relax.vrma";
const IDLE_SLEEPY_VRMA_PATH: &str = "animations/VRMA/Sleepy.vrma";
const IDLE_BLUSH_VRMA_PATH: &str = "animations/VRMA/Blush.vrma";

// Phase 5.5 menu-triggered animation library.
// Only these five are preloaded; the hundreds of other downloaded VRMAs stay
// as stock and consume no runtime animation slots yet.
const MENU_WAVE_VRMA_PATH: &str = "animations/Clawatar_animations/161_Waving.vrma";
const MENU_THINKING_VRMA_PATH: &str = "animations/VRMA/Thinking.vrma";
const MENU_ANGRY_VRMA_PATH: &str = "animations/VRMA/Angry.vrma";
const MENU_SAD_VRMA_PATH: &str = "animations/VRMA/Sad.vrma";
const MENU_SURPRISED_VRMA_PATH: &str = "animations/VRMA/Surprised.vrma";
pub(crate) fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("Nera Camera"),
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Msaa::Off,
        Transform::from_xyz(0.0, 0.8, 4.5).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
}

pub(crate) fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        Name::new("Nera Key Light"),
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, -0.5, 0.0)),
    ));
}

// -----------------------------------------------------------------------------
// Nera Spawning
// -----------------------------------------------------------------------------

pub(crate) fn spawn_nera(mut commands: Commands, asset_server: Res<AssetServer>) {
    info!("Loading Nera VRM 1.0...");

    let target = commands
        .spawn((
            Name::new("LookAt Target"),
            LookAtTarget,
            Transform::from_xyz(0.0, 1.0, 2.0),
        ))
        .id();

    commands
        .spawn((
            Name::new("Nera"),
            NeraAvatar,
            VrmHandle(asset_server.load(VRM_PATH)),
            LookAt::Target(target),
            Transform::from_xyz(0.0, -0.85, 0.0),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Name::new("Nera Idle"),
                    VrmaHandle(asset_server.load(IDLE_VRMA_PATH)),
                ))
                .observe(register_idle_animation);

            parent
                .spawn((
                    Name::new("Nera Floating"),
                    VrmaHandle(asset_server.load(FLOATING_VRMA_PATH)),
                ))
                .observe(register_floating_animation);

            parent
                .spawn((
                    Name::new("Nera Death Reaction"),
                    VrmaHandle(asset_server.load(DEATH_VRMA_PATH)),
                ))
                .observe(register_death_animation);

            parent
                .spawn((
                    Name::new("Nera Falling"),
                    VrmaHandle(asset_server.load(FALLING_VRMA_PATH)),
                ))
                .observe(register_falling_animation);

            parent
                .spawn((
                    Name::new("Nera Falling To Landing"),
                    VrmaHandle(asset_server.load(FALLING_TO_LANDING_VRMA_PATH)),
                ))
                .observe(register_falling_to_landing_animation);

            parent
                .spawn((
                    Name::new("Nera Hit Head"),
                    VrmaHandle(asset_server.load(HIT_HEAD_VRMA_PATH)),
                ))
                .observe(register_hit_head_animation);

            parent
                .spawn((
                    Name::new("Nera Hit Chest"),
                    VrmaHandle(asset_server.load(HIT_CHEST_VRMA_PATH)),
                ))
                .observe(register_hit_chest_animation);

            parent
                .spawn((
                    Name::new("Nera Head Stroke Reaction"),
                    VrmaHandle(asset_server.load(HEAD_STROKE_VRMA_PATH)),
                ))
                .observe(register_head_stroke_animation);

            parent
                .spawn((
                    Name::new("Nera Praying Menu Pose"),
                    VrmaHandle(asset_server.load(PRAYING_VRMA_PATH)),
                ))
                .observe(register_praying_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Enter"),
                    VrmaHandle(asset_server.load(SITTING_ENTER_VRMA_PATH)),
                ))
                .observe(register_sitting_enter_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Loop"),
                    VrmaHandle(asset_server.load(SITTING_LOOP_VRMA_PATH)),
                ))
                .observe(register_sitting_loop_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Exit"),
                    VrmaHandle(asset_server.load(SITTING_EXIT_VRMA_PATH)),
                ))
                .observe(register_sitting_exit_animation);

            // Phase 5.8 idle micro-actions. Same child-of-"Nera" pattern as
            // everything else here: bevy_vrm1 needs the VRM ancestry to
            // retarget the clip.
            parent
                .spawn((
                    Name::new("Nera Idle LookAround"),
                    VrmaHandle(asset_server.load(IDLE_LOOK_AROUND_VRMA_PATH)),
                ))
                .observe(register_idle_look_around_animation);

            parent
                .spawn((
                    Name::new("Nera Idle Relax"),
                    VrmaHandle(asset_server.load(IDLE_RELAX_VRMA_PATH)),
                ))
                .observe(register_idle_relax_animation);

            parent
                .spawn((
                    Name::new("Nera Idle Sleepy"),
                    VrmaHandle(asset_server.load(IDLE_SLEEPY_VRMA_PATH)),
                ))
                .observe(register_idle_sleepy_animation);

            parent
                .spawn((
                    Name::new("Nera Idle Blush"),
                    VrmaHandle(asset_server.load(IDLE_BLUSH_VRMA_PATH)),
                ))
                .observe(register_idle_blush_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Wave"),
                    VrmaHandle(asset_server.load(MENU_WAVE_VRMA_PATH)),
                ))
                .observe(register_wave_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Thinking"),
                    VrmaHandle(asset_server.load(MENU_THINKING_VRMA_PATH)),
                ))
                .observe(register_thinking_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Angry"),
                    VrmaHandle(asset_server.load(MENU_ANGRY_VRMA_PATH)),
                ))
                .observe(register_angry_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Sad"),
                    VrmaHandle(asset_server.load(MENU_SAD_VRMA_PATH)),
                ))
                .observe(register_sad_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Surprised"),
                    VrmaHandle(asset_server.load(MENU_SURPRISED_VRMA_PATH)),
                ))
                .observe(register_surprised_animation);
        });
}
