//! Real-file export validation through the same VRMA asset loader as Nera.
//! No window, renderer, VRM model, or application runner is started.

use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bevy::{
    animation::AnimationEntityMut, asset::LoadState, gltf::GltfPlugin, mesh::MeshPlugin,
    prelude::*, window::RequestRedraw, world_serialization::WorldSerializationPlugin,
};
use bevy_vrm1::prelude::{VrmaAsset, VrmaPlugin};

use super::combine::{Segment, combine};

struct TemporaryLibrary(PathBuf);

impl TemporaryLibrary {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nera-combine-loader-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for TemporaryLibrary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn loader_app(library: &TemporaryLibrary) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: library.0.to_string_lossy().into_owned(),
            ..default()
        },
        WorldSerializationPlugin,
        MeshPlugin,
        GltfPlugin::default(),
    ))
    .init_asset::<AnimationClip>()
    .init_asset::<AnimationGraph>()
    .add_message::<RequestRedraw>()
    .add_plugins(VrmaPlugin);
    app.finish();
    app.cleanup();
    app
}

fn load_clip(app: &mut App, path: String) -> AnimationClip {
    // Loading VrmaAsset invokes bevy_vrm1's actual .vrma AssetLoader, which
    // delegates glTF parsing and animation-curve construction to Bevy.
    let server = app.world().resource::<AssetServer>().clone();
    let handle: Handle<VrmaAsset> = server.load(path);
    let timeout = Instant::now() + Duration::from_secs(20);
    loop {
        app.update();
        match server.get_load_state(handle.id()) {
            Some(LoadState::Failed(error)) => panic!("VRMA loader rejected export: {error}"),
            Some(LoadState::Loaded) if server.is_loaded_with_dependencies(handle.id()) => {
                let assets = app.world().resource::<Assets<VrmaAsset>>();
                if let Some(asset) = assets.get(&handle) {
                    assert!(!asset.gltf.nodes.is_empty(), "Export has no skeleton");
                    assert!(!asset.gltf.scenes.is_empty(), "Export has no scene");
                    assert_eq!(asset.gltf.animations.len(), 1, "Expected one joined clip");
                    let source = asset.gltf.source.as_ref().expect("VRMA source retained");
                    assert!(
                        source
                            .document
                            .extension_value("VRMC_vrm_animation")
                            .is_some(),
                        "Export lost its VRMA humanoid extension"
                    );
                    if let Some(clip) = app
                        .world()
                        .resource::<Assets<AnimationClip>>()
                        .get(&asset.gltf.animations[0])
                    {
                        return clip.clone();
                    }
                }
            }
            _ => {}
        }
        assert!(Instant::now() < timeout, "VRMA asset load timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_curves_evaluate_across_joins(app: &mut App, clip: &AnimationClip) {
    assert!(
        !clip.curves().is_empty(),
        "Loader produced no animation curves"
    );
    let dummy = app.world_mut().spawn(Transform::default()).id();
    let mut query = app.world_mut().query::<AnimationEntityMut>();
    let graph_node = AnimationNodeIndex::new(0);
    for curves in clip.curves().values() {
        for curve in curves {
            let domain = curve.0.domain();
            assert!(domain.start().is_finite() && domain.end().is_finite());
            for time in [0.0, 0.25, 0.499, 0.5, 0.501, 0.999, 1.0, 1.5, 2.0] {
                assert!(
                    time >= domain.start() - 0.0001 && time <= domain.end() + 0.0001,
                    "Exported curve does not cover the joined clip"
                );
                *app.world_mut().get_mut::<Transform>(dummy).unwrap() = Transform::default();
                let mut evaluator = curve.0.create_evaluator();
                curve
                    .0
                    .apply(&mut *evaluator, time, 1.0, graph_node)
                    .expect("Exported animation sample cannot be evaluated");
                evaluator
                    .commit(query.get_mut(app.world_mut(), dummy).unwrap())
                    .expect("Exported animation sample cannot be applied");
                let transform = app.world().get::<Transform>(dummy).unwrap();
                assert!(transform.translation.is_finite());
                assert!(transform.rotation.is_finite());
                assert!((transform.rotation.length() - 1.0).abs() < 0.002);
                assert!(transform.scale.is_finite());
            }
        }
    }
    app.world_mut().despawn(dummy);
}

#[test]
#[ignore = "requires the local real VRMA library; loads assets without a window or renderer"]
fn combined_real_libraries_load_with_vrma_loader_without_rendering() {
    const KEYS: [&str; 4] = [
        "animasi_basic/idle.vrma",
        "animasi_basic/LookAround.vrma",
        "Clawatar_animations/161_Waving.vrma",
        "Mixamo-VRMA/Agreeing.vrma",
    ];
    let source_root = PathBuf::from(crate::nera_asset_path()).join("animations");
    let library = TemporaryLibrary::new();
    let mut originals = Vec::new();
    for key in KEYS {
        let source = source_root.join(key);
        let bytes = fs::read(&source)
            .unwrap_or_else(|error| panic!("Required real fixture {}: {error}", source.display()));
        let destination = library.0.join(key);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, &bytes).unwrap();
        originals.push((destination, bytes));
    }

    let mut app = loader_app(&library);
    // Use two different first-source skeletons so the loader verifies exports
    // beginning with both the Mixamo rest pose and the normalized UniGLTF rig.
    for (name, order) in [
        ("mixed_mixamo_first", [0, 1, 2, 3]),
        ("mixed_unigltf_first", [1, 3, 0, 2]),
    ] {
        let segments: Vec<_> = order
            .into_iter()
            .map(|index| Segment {
                key: KEYS[index].to_owned(),
                start: 0.1,
                end: Some(0.6),
            })
            .collect();
        let output = combine(&library.0, &segments, name).expect("Combine real VRMAs");
        let relative = output
            .strip_prefix(&library.0)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let clip = load_clip(&mut app, relative);
        assert!((clip.duration() - 2.0).abs() < 0.0001);
        assert_curves_evaluate_across_joins(&mut app, &clip);
    }

    for (path, original) in originals {
        assert_eq!(fs::read(path).unwrap(), original, "Source bytes changed");
    }
    // Release the asset reader and its handles before deleting the temp library.
    drop(app);
}
