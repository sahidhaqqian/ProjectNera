//! Temporary activity props. The existing animation/placement owners move Nera.
use super::{FocusPreset, FocusSession};
use crate::{
    animation::{NeraAnimationState, VisualState},
    avatar::NeraAvatar,
    interaction::NeraLiveBone,
};
use bevy::{
    prelude::*,
    transform::TransformSystems,
    world_serialization::{WorldAsset, WorldAssetRoot},
};
use bevy_vrm1::prelude::{RestGlobalTransform, VrmBone};

pub(super) const BOOK_PATH: &str = "props/focus/study-book.glb";
pub(super) const LAPTOP_PATH: &str = "props/focus/work-laptop.glb";
#[derive(Resource, Default)]
struct ActivityAssets([Handle<WorldAsset>; 2]);
#[derive(Component)]
pub(crate) struct FocusProp {
    pub(crate) preset: FocusPreset,
    avatar: Entity,
    body_mode: crate::avatar::body_mode::AvatarBodyMode,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<ActivityAssets>()
        .add_systems(
            Startup,
            |server: Res<AssetServer>, mut assets: ResMut<ActivityAssets>| {
                assets.0 = [
                    server.load(format!("{BOOK_PATH}#Scene0")),
                    server.load(format!("{LAPTOP_PATH}#Scene0")),
                ];
            },
        )
        .add_systems(PostUpdate, update_props.after(TransformSystems::Propagate));
}

fn book_grip(chest_to_head: Vec3, across: Vec3, hands: Vec3) -> Option<Transform> {
    let length = chest_to_head.length();
    let up = chest_to_head.try_normalize()?;
    let side = (across - up * across.dot(up)).try_normalize()?;
    let forward = side.cross(up).try_normalize()?;
    if !length.is_finite() || length < 1e-6 || !hands.is_finite() {
        return None;
    }
    let rotation =
        Quat::from_mat3(&Mat3::from_cols(side, up, forward)) * Quat::from_rotation_x(-0.85);
    // The downloaded open book is 0.873m wide after its authored scene scale.
    let scale = across.length() * 1.2 / 0.873;
    Some(Transform {
        translation: hands + forward * across.length() * 0.3 - up * across.length() * 0.05,
        rotation,
        scale: Vec3::splat(scale),
    })
}

fn desk_transform(rest_leg_height: f32, body_ratio: f32, hands: Vec3) -> Option<Transform> {
    if !rest_leg_height.is_finite()
        || rest_leg_height < 1e-5
        || !body_ratio.is_finite()
        || body_ratio < 1e-5
        || !hands.is_finite()
    {
        return None;
    }
    // Use a medium workstation size from standing rest proportions so
    // bending the knees while sitting never shrinks the desk.
    // Source height and keyboard surface measured from the unchanged GLB.
    // Align the keyboard to the evaluated typing fingers, independent of seat height.
    let scale = rest_leg_height * 0.85 / 0.560870 * body_ratio;
    // Screen faces Nera (+Z); the laptop's back faces the desktop viewer.
    let rotation = Quat::IDENTITY;
    Some(Transform {
        translation: hands - rotation * (Vec3::new(0.0, 0.315, -0.04) * scale),
        rotation,
        scale: Vec3::splat(scale),
    })
}

fn update_props(
    time: Res<Time<Real>>,
    focus: Res<FocusSession>,
    animation: Res<NeraAnimationState>,
    clips: Res<crate::animation::NeraAnimationClips>,
    playback: Res<crate::animation::NeraAnimationPlayback>,
    assets: Res<ActivityAssets>,
    scenes: Res<Assets<WorldAsset>>,
    avatars: Query<(Entity, &GlobalTransform), With<NeraAvatar>>,
    bones: Query<(Entity, &VrmBone, &RestGlobalTransform, &GlobalTransform), With<NeraLiveBone>>,
    mut props: Query<(Entity, &FocusProp, &mut Transform)>,
    body_mode: Res<crate::avatar::body_mode::AvatarBodyMode>,
    environment: Res<crate::environment::DesktopEnvironment>,
    cloud: Res<crate::cloud::CloudRuntime>,
    mut commands: Commands,
    mut warned: Local<Option<Entity>>,
    mut settling: Local<(Option<FocusPreset>, f32)>,
) {
    let avatar = avatars.single().ok();
    let active = match animation.visual_state() {
        VisualState::FocusActivity(preset)
            if focus.running_preset() == Some(preset)
                && (preset == FocusPreset::Study
                    || cloud.is_riding()
                    || matches!(
                        environment.placement,
                        crate::environment::PlacementState::Taskbar(_)
                    )) =>
        {
            Some(preset)
        }
        _ => None,
    };
    let active = active.filter(|preset| {
        *preset != FocusPreset::Work || clips.focus_activity_playing(*preset, &playback)
    });
    if settling.0 != active {
        *settling = (active, 0.0);
    }
    settling.1 += time.delta_secs().min(0.1);
    let find = |name| bones.iter().find(|(_, bone, _, _)| bone.as_str() == name);
    let book_placement = || {
        find("chest")
            .or_else(|| find("spine"))
            .and_then(|(chest, _, rest, global)| {
                let inverse = rest.0.to_matrix().inverse();
                let point = |name| {
                    find(name).map(|(_, _, rest, _)| inverse.transform_point3(rest.0.translation()))
                };
                let hands = global.to_matrix().inverse().transform_point3(
                    (find("leftHand")?.3.translation() + find("rightHand")?.3.translation()) * 0.5,
                );
                Some((
                    chest,
                    book_grip(
                        point("head")?,
                        point("leftUpperArm")? - point("rightUpperArm")?,
                        hands,
                    )?,
                ))
            })
    };
    let mut existing = false;
    for (entity, prop, mut transform) in &mut props {
        if active != Some(prop.preset)
            || avatar.map(|(entity, _)| entity) != Some(prop.avatar)
            || prop.body_mode != *body_mode
        {
            commands.entity(entity).despawn();
        } else {
            if prop.preset == FocusPreset::Study {
                // Follow the evaluated hands through the authored pose/blend;
                // only this temporary prop transform is written.
                if let Some((_, next)) = book_placement().filter(|(_, t)| t.to_matrix().is_finite())
                {
                    *transform = next;
                }
            }
            existing = true;
        }
    }
    let (Some(preset), Some((avatar, avatar_global))) = (active, avatar) else {
        return;
    };
    if existing {
        return;
    }
    if preset == FocusPreset::Work && settling.1 < 0.35 {
        return;
    }
    let index = usize::from(preset == FocusPreset::Work);
    if !scenes.contains(assets.0[index].id()) {
        return;
    }
    let placement = match preset {
        FocusPreset::Study => book_placement(),
        FocusPreset::Work => (|| {
            let inverse = avatar_global.to_matrix().inverse();
            let point = |name| {
                find(name).map(|(_, _, _, global)| inverse.transform_point3(global.translation()))
            };
            let left = point("leftIndexProximal").or_else(|| point("leftHand"))?;
            let right = point("rightIndexProximal").or_else(|| point("rightHand"))?;
            let rest = |name| {
                find(name).map(|(_, _, rest, _)| inverse.transform_point3(rest.0.translation()))
            };
            let rest_shoulders = rest("leftUpperArm")?.distance(rest("rightUpperArm")?);
            let rest_feet = (rest("leftFoot")? + rest("rightFoot")?) * 0.5;
            Some((
                avatar,
                desk_transform(
                    rest("hips")?.y - rest_feet.y,
                    point("leftUpperArm")?.distance(point("rightUpperArm")?) / rest_shoulders,
                    (left + right) * 0.5,
                )?,
            ))
        })(),
    };
    let Some((parent, transform)) =
        placement.filter(|(_, transform)| transform.to_matrix().is_finite())
    else {
        if *warned != Some(avatar) {
            warn!(
                "Focus {:?}: prop unavailable for this humanoid rig; animation still works",
                preset
            );
            *warned = Some(avatar);
        }
        return;
    };
    commands.spawn((
        Name::new(format!("Focus/{preset:?} prop")),
        FocusProp {
            preset,
            avatar,
            body_mode: *body_mode,
        },
        WorldAssetRoot(assets.0[index].clone()),
        transform,
        ChildOf(parent),
    ));
    info!("Focus {preset:?} prop attached");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_props_use_rig_geometry_and_reject_degenerate_rigs() {
        let book = book_grip(Vec3::Y * 0.08, Vec3::X * 0.04, Vec3::ZERO).unwrap();
        assert!(book.to_matrix().is_finite() && book.scale.x > 0.0);
        assert!(
            (book.rotation * Vec3::Y).dot(Vec3::Z) < 0.0,
            "pages face Nera"
        );
        assert!(book_grip(Vec3::ZERO, Vec3::X, Vec3::ZERO).is_none());
        let fingers = Vec3::new(0.0, 0.8, 0.25);
        let desk = desk_transform(0.8, 1.0, fingers).unwrap();
        let chibi = desk_transform(0.8, 0.62, fingers).unwrap();
        assert!(chibi.scale.x < desk.scale.x);
        assert_eq!(desk.rotation, Quat::IDENTITY, "laptop screen faces Nera");
        assert!(desk_transform(0.0, 1.0, Vec3::ZERO).is_none());
        assert!((desk.transform_point(Vec3::new(0.0, 0.315, -0.04)) - fingers).length() < 1e-6);
    }
    #[test]
    fn study_book_tracks_evaluated_hands_without_accumulating_offsets() {
        let start = book_grip(Vec3::Y * 0.25, Vec3::X * 0.3, Vec3::Z * 0.1).unwrap();
        let shift = Vec3::new(0.01, 0.15, 0.03);
        let moved = book_grip(Vec3::Y * 0.25, Vec3::X * 0.3, Vec3::Z * 0.1 + shift).unwrap();
        assert!((moved.translation - start.translation - shift).length() < 1e-6);
        assert_eq!(start.scale, moved.scale);
        assert_eq!(start.rotation, moved.rotation);
        for _ in 0..100 {
            assert_eq!(
                book_grip(Vec3::Y * 0.25, Vec3::X * 0.3, Vec3::Z * 0.1).unwrap(),
                start
            );
        }
    }
    #[test]
    fn focus_assets_are_bundled_small_plain_gltf() {
        for path in [BOOK_PATH, LAPTOP_PATH] {
            let bytes = std::fs::read(crate::nera_asset_file(path)).unwrap();
            assert_eq!(&bytes[..4], b"glTF");
            let budget = if path == BOOK_PATH {
                50_000
            } else {
                12_000_000
            };
            assert!(bytes.len() < budget);
            let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            let json: serde_json::Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
            assert!(
                json["extensionsRequired"]
                    .as_array()
                    .is_none_or(|required| required.is_empty())
            );
            assert!(
                json["images"]
                    .as_array()
                    .is_none_or(|images| images.iter().all(|image| image.get("uri").is_none()))
            );
            if path == LAPTOP_PATH {
                assert_eq!(
                    bytes.len(),
                    11_612_308,
                    "retain the uncompressed supplied GLB"
                );
                let triangles: u64 = json["meshes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
                    .map(|primitive| {
                        let accessor = primitive["indices"].as_u64().unwrap() as usize;
                        json["accessors"][accessor]["count"].as_u64().unwrap() / 3
                    })
                    .sum();
                assert_eq!(
                    triangles, 267_920,
                    "retain the supplied workstation topology"
                );
                let binary = &bytes[20 + length + 8..];
                let images = json["images"].as_array().unwrap();
                assert_eq!(images.len(), 3);
                for image in images {
                    let view = &json["bufferViews"][image["bufferView"].as_u64().unwrap() as usize];
                    let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
                    let png =
                        &binary[offset..offset + view["byteLength"].as_u64().unwrap() as usize];
                    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
                    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 1024);
                    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 1024);
                }
            }
        }
    }
}
