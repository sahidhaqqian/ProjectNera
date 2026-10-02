//! Named facial morphs keep brows, eyelids and mouth independent of body animation.
use super::NeraExpressionPreset;
use bevy::camera::visibility::RenderLayers;
use bevy::{mesh::morph::MorphWeights, prelude::*};
use bevy_vrm1::prelude::HeadBoneEntity;
use bevy_vrm1::prelude::VrmSystemSets;

#[derive(Resource, Default)]
struct MorphNameCache(std::collections::HashMap<AssetId<Mesh>, Vec<String>>);

#[derive(Component, Clone, Copy)]
pub(crate) struct FacePose(pub(crate) Option<(NeraExpressionPreset, f32)>);

#[derive(Component, Default)]
pub(crate) struct AutomaticAngry(pub(crate) f32);

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct AutomaticFace {
    pub(crate) previous: Option<(NeraExpressionPreset, f32)>,
    pub(crate) current: Option<(NeraExpressionPreset, f32)>,
    blend: f32,
}

impl AutomaticFace {
    pub(crate) fn settled(selection: Option<(NeraExpressionPreset, f32)>) -> Self {
        Self {
            previous: selection,
            current: selection,
            blend: 1.0,
        }
    }
    pub(crate) fn advance(&mut self, selection: Option<(NeraExpressionPreset, f32)>, dt: f32) {
        if selection != self.current {
            self.previous = self.current;
            self.current = selection;
            self.blend = 0.0;
        }
        self.blend = (self.blend + dt.max(0.0) / 0.18).min(1.0);
        if self.blend >= 1.0 {
            self.previous = self.current;
        }
    }
    fn selections(&self) -> impl Iterator<Item = (NeraExpressionPreset, f32)> {
        [
            (self.previous, 1.0 - self.blend),
            (self.current, self.blend),
        ]
        .into_iter()
        .filter_map(|(selection, weight)| {
            selection.map(|(preset, intensity)| (preset, intensity * weight))
        })
    }
    pub(crate) fn weights<'a>(
        &self,
        available: impl Iterator<Item = &'a str>,
    ) -> Vec<(String, f32)> {
        let available: Vec<_> = available.collect();
        let mut weights = std::collections::BTreeMap::<String, f32>::new();
        for (preset, intensity) in self.selections() {
            for (name, value) in
                super::presets::preset_weights(preset, intensity, available.iter().copied())
            {
                *weights.entry(name).or_default() += value;
            }
        }
        weights
            .into_iter()
            .map(|(name, value)| (name, value.min(1.0)))
            .collect()
    }
}

#[derive(Component)]
struct FacialMorphs {
    root: Entity,
    targets: Vec<(String, usize)>,
    applied: Vec<usize>,
}

#[derive(Component)]
struct BlushBuilt;
#[derive(Component)]
struct FaceBlush {
    root: Entity,
    opacity: f32,
}

pub(crate) fn install(app: &mut App) {
    #[cfg(debug_assertions)]
    app.add_systems(Last, record_face_smoke);
    app.init_resource::<MorphNameCache>().add_systems(
        Last,
        cache_morph_names.before(crate::memory::release_avatar_mesh_copies),
    );
    app.add_systems(
        PostUpdate,
        (cache_facial_morphs, reset_facial_morphs)
            .chain()
            .before(VrmSystemSets::Expressions),
    )
    .add_systems(
        PostUpdate,
        apply_facial_morphs
            .after(VrmSystemSets::Expressions)
            .before(bevy::transform::TransformSystems::Propagate),
    );
    app.add_systems(Update, (prepare_blush, update_blush).chain());
}

fn prepare_blush(
    faces: Query<&FacialMorphs>,
    roots: Query<(Entity, &HeadBoneEntity), (With<FacePose>, Without<BlushBuilt>)>,
    layers: Query<&RenderLayers>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (root, head) in &roots {
        if !faces.iter().any(|face| face.root == root) {
            continue;
        }
        commands.entity(root).insert(BlushBuilt);
        let mesh = meshes.add(Sphere::new(1.0));
        for side in [-1.0, 1.0] {
            for (radius, opacity, depth) in [(1.2, 0.09, 0.0), (1.0, 0.18, 0.001)] {
                let material = materials.add(StandardMaterial {
                    base_color: Color::srgba(0.98, 0.22, 0.32, opacity),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                });
                commands.spawn((
                    FaceBlush { root, opacity },
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(side * 0.048, 0.033, 0.074 + depth).with_scale(Vec3::new(
                        0.022 * radius,
                        0.009 * radius,
                        0.002,
                    )),
                    Visibility::Hidden,
                    layers.get(head.0).cloned().unwrap_or_default(),
                    ChildOf(head.0),
                ));
            }
        }
    }
}

fn update_blush(
    poses: Query<(&FacePose, Option<&AutomaticFace>)>,
    mut cheeks: Query<(
        &FaceBlush,
        &MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (cheek, material, mut visibility) in &mut cheeks {
        let selection = poses.get(cheek.root).ok().and_then(|(pose, automatic)| {
            pose.0.or_else(|| automatic.and_then(|face| face.current))
        });
        let strength = selection.map_or(0.0, |(preset, intensity)| match preset {
            NeraExpressionPreset::Embarrassed => intensity,
            NeraExpressionPreset::Timid => intensity * 0.6,
            _ => 0.0,
        });
        *visibility = if strength > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(mut material) = materials.get_mut(&material.0) {
            material
                .base_color
                .set_alpha(cheek.opacity * strength.clamp(0.0, 1.0));
        }
    }
}

fn cache_morph_names(
    mut events: MessageReader<AssetEvent<Mesh>>,
    meshes: Res<Assets<Mesh>>,
    mut cache: ResMut<MorphNameCache>,
) {
    for event in events.read() {
        match event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => {
                if let Some(names) = meshes
                    .get(*id)
                    .and_then(|mesh| mesh.try_morph_target_names().ok().flatten())
                {
                    cache.0.insert(*id, names.to_vec());
                }
            }
            AssetEvent::Removed { id } => {
                cache.0.remove(id);
            }
            _ => {}
        }
    }
}

fn cache_facial_morphs(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    cache: Res<MorphNameCache>,
    morphs: Query<(Entity, &MorphWeights), Without<FacialMorphs>>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<FacePose>>,
) {
    for (entity, weights) in &morphs {
        let Some(handle) = weights.first_mesh() else {
            continue;
        };
        let Some(names) = cache.0.get(&handle.id()).map(Vec::as_slice).or_else(|| {
            meshes
                .get(handle)
                .and_then(|mesh| mesh.try_morph_target_names().ok().flatten())
        }) else {
            continue;
        };
        if !names.iter().any(|name| name == "Fcl_BRW_Angry") {
            continue;
        }
        let mut root = entity;
        while !roots.contains(root) {
            let Ok(parent) = parents.get(root) else {
                break;
            };
            root = parent.parent();
        }
        if !roots.contains(root) {
            continue;
        }
        let targets = names
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                name.starts_with("Fcl_ALL_")
                    || name.starts_with("Fcl_BRW_")
                    || name.starts_with("Fcl_EYE_")
                    || name.starts_with("Fcl_MTH_")
            })
            .map(|(index, name)| (name.clone(), index))
            .collect();
        commands.entity(entity).insert(FacialMorphs {
            root,
            targets,
            applied: Vec::new(),
        });
    }
}

fn reset_facial_morphs(mut morphs: Query<(&mut MorphWeights, &mut FacialMorphs)>) {
    for (mut weights, mut face) in &mut morphs {
        for index in face.applied.drain(..) {
            if let Some(weight) = weights.weights_mut().get_mut(index) {
                *weight = 0.0;
            }
        }
    }
}

pub(crate) fn recipe(preset: NeraExpressionPreset) -> &'static [(&'static str, f32)] {
    use NeraExpressionPreset::*;
    match preset {
        Default => &[],
        Joy => &[
            ("Fcl_BRW_Joy", 0.4),
            ("Fcl_EYE_Joy", 0.12),
            ("Fcl_MTH_Joy", 0.7),
        ],
        Sad => &[
            ("Fcl_BRW_Sorrow", 1.0),
            ("Fcl_EYE_Sorrow", 0.55),
            ("Fcl_MTH_Sorrow", 0.55),
            ("Fcl_MTH_A", 0.12),
        ],
        Angry => &[
            ("Fcl_ALL_Angry", 0.45),
            ("Fcl_BRW_Angry", 1.0),
            ("Fcl_EYE_Angry", 0.25),
            ("Fcl_MTH_Angry", 0.55),
            ("Fcl_MTH_Sorrow", 0.2),
        ],
        Embarrassed => &[
            ("Fcl_BRW_Sorrow", 1.0),
            ("Fcl_EYE_Sorrow", 0.25),
            ("Fcl_MTH_Down", 0.3),
            ("Fcl_MTH_A", 0.18),
        ],
        Timid => &[
            ("Fcl_BRW_Joy", 0.45),
            ("Fcl_EYE_Joy", 1.0),
            ("Fcl_MTH_Up", 0.75),
        ],
        Surprised => &[
            ("Fcl_BRW_Angry", 0.55),
            ("Fcl_BRW_Surprised", 0.45),
            ("Fcl_EYE_Surprised", 0.95),
            ("Fcl_MTH_O", 0.45),
        ],
        Exhausted => &[
            ("Fcl_BRW_Fun", 0.3),
            ("Fcl_EYE_Close", 1.0),
            ("Fcl_MTH_O", 0.7),
        ],
    }
}

#[cfg(debug_assertions)]
fn record_face_smoke(
    time: Res<Time>,
    mut recorded: Local<bool>,
    morphs: Query<(&MorphWeights, &FacialMorphs)>,
    poses: Query<&FacePose>,
) {
    if *recorded
        || time.elapsed_secs() < 11.0
        || !std::env::args().any(|arg| arg == "--expression-smoke")
    {
        return;
    }
    *recorded = true;
    let report: Vec<_> = morphs
        .iter()
        .map(|(weights, face)| {
            let pose = poses
                .get(face.root)
                .ok()
                .and_then(|pose| pose.0)
                .map(|(preset, _)| preset.name());
            let values: Vec<_> = face
                .targets
                .iter()
                .filter_map(|(name, index)| {
                    let weight = weights.weights().get(*index).copied().unwrap_or_default();
                    (weight > 0.0).then(|| serde_json::json!({"name":name,"weight":weight}))
                })
                .collect();
            serde_json::json!({"pose":pose,"targets":face.targets.len(),"weights":values})
        })
        .collect();
    let directory = std::env::temp_dir().join("nera-feature-smoke");
    let _ = std::fs::create_dir_all(&directory);
    let _ = std::fs::write(
        directory.join("face-morphs.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
}

fn apply_facial_morphs(
    poses: Query<(&FacePose, Option<&AutomaticAngry>, Option<&AutomaticFace>)>,
    mut morphs: Query<(&mut MorphWeights, &mut FacialMorphs)>,
) {
    for (mut weights, mut face) in &mut morphs {
        let Ok((pose, automatic, facial)) = poses.get(face.root) else {
            continue;
        };
        let manual = pose.0.is_some();
        let selection = pose.0.or_else(|| {
            automatic
                .filter(|angry| angry.0 > 0.0)
                .map(|angry| (NeraExpressionPreset::Angry, angry.0))
        });
        let detailed =
            !manual && facial.is_some_and(|face| face.current.is_some() || face.previous.is_some());
        if selection.is_none() && !detailed {
            continue;
        }
        let selections: Vec<_> = if detailed {
            facial.unwrap().selections().collect()
        } else {
            selection.into_iter().collect()
        };
        let targets = face.targets.clone();
        // Automatic basic faces replace aggregate emotions but retain natural blinks.
        if manual || detailed {
            for (name, index) in &targets {
                if !manual
                    && (name == "Fcl_EYE_Close"
                        || name == "Fcl_EYE_Close_L"
                        || name == "Fcl_EYE_Close_R")
                {
                    continue;
                }
                if let Some(weight) = weights.weights_mut().get_mut(*index) {
                    *weight = 0.0;
                }
            }
        }
        let mut applied = Vec::new();
        let mut values = std::collections::BTreeMap::<&str, f32>::new();
        for (preset, intensity) in selections {
            let intensity = if intensity.is_finite() {
                intensity.clamp(0.0, 1.0)
            } else {
                0.0
            };
            for (name, value) in recipe(preset) {
                *values.entry(name).or_default() += value * intensity;
            }
        }
        for (name, value) in values {
            if let Some((_, index)) = targets.iter().find(|(target, _)| target == name)
                && let Some(slot) = weights.weights_mut().get_mut(*index)
            {
                *slot = if !manual && name == "Fcl_EYE_Close" {
                    slot.max(value)
                } else {
                    value
                };
                applied.push(*index);
            }
        }
        face.applied = if manual || detailed {
            targets.into_iter().map(|(_, index)| index).collect()
        } else {
            applied
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_faces_fade_and_release_ownership_for_unmapped_clips() {
        let mut face = AutomaticFace::default();
        face.advance(Some((NeraExpressionPreset::Angry, 0.9)), 0.09);
        assert_eq!(
            face.weights(["angry"].into_iter()),
            vec![("angry".into(), 0.45)]
        );
        face.advance(Some((NeraExpressionPreset::Angry, 0.9)), 0.09);
        assert_eq!(
            face.weights(["angry"].into_iter()),
            vec![("angry".into(), 0.9)]
        );
        face.advance(None, 0.18);
        assert!(face.current.is_none() && face.previous.is_none());
        assert!(face.weights(["angry"].into_iter()).is_empty());
    }
    #[test]
    fn reference_faces_have_distinct_eyes_and_mouths() {
        assert!(recipe(NeraExpressionPreset::Timid).contains(&("Fcl_EYE_Joy", 1.0)));
        assert!(recipe(NeraExpressionPreset::Exhausted).contains(&("Fcl_EYE_Close", 1.0)));
        assert!(
            recipe(NeraExpressionPreset::Exhausted)
                .iter()
                .any(|(name, _)| *name == "Fcl_MTH_O")
        );
        assert!(
            recipe(NeraExpressionPreset::Embarrassed)
                .iter()
                .any(|(name, _)| *name == "Fcl_BRW_Sorrow")
        );
        assert!(recipe(NeraExpressionPreset::Angry).contains(&("Fcl_BRW_Angry", 1.0)));
        assert!(
            !recipe(NeraExpressionPreset::Angry)
                .iter()
                .any(|(name, _)| *name == "Fcl_EYE_Close")
        );
    }
    #[test]
    fn releasing_a_face_clears_unbound_morphs() {
        let mut app = App::new();
        app.add_systems(Update, (reset_facial_morphs, apply_facial_morphs).chain());
        let root = app
            .world_mut()
            .spawn(FacePose(Some((NeraExpressionPreset::Timid, 1.0))))
            .id();
        let mesh = app
            .world_mut()
            .spawn((
                MorphWeights::new(vec![0.0], None).unwrap(),
                FacialMorphs {
                    root,
                    targets: vec![("Fcl_EYE_Joy".into(), 0)],
                    applied: Vec::new(),
                },
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights(),
            &[1.0]
        );
        app.world_mut().get_mut::<FacePose>(root).unwrap().0 = None;
        app.update();
        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights(),
            &[0.0]
        );
    }

    #[test]
    fn automatic_anger_keeps_blinks_and_manual_preview_wins() {
        let mut app = App::new();
        app.add_systems(Update, apply_facial_morphs);
        let root = app
            .world_mut()
            .spawn((FacePose(None), AutomaticAngry(1.0)))
            .id();
        let mesh = app
            .world_mut()
            .spawn((
                MorphWeights::new(vec![0.9, 0.0, 0.4], None).unwrap(),
                FacialMorphs {
                    root,
                    targets: vec![
                        ("Fcl_EYE_Close".into(), 0),
                        ("Fcl_BRW_Angry".into(), 1),
                        ("Fcl_ALL_Happy".into(), 2),
                    ],
                    applied: Vec::new(),
                },
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights(),
            &[0.9, 1.0, 0.4]
        );
        app.world_mut().get_mut::<FacePose>(root).unwrap().0 =
            Some((NeraExpressionPreset::Default, 1.0));
        app.update();
        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights(),
            &[0.0, 0.0, 0.0]
        );
    }
}
