//! Remove only an owned import, after live users have safely switched/exited.
use super::super::{ActiveCharacter, CharacterImportStatus, RestoreDefaultCharacter};
use super::*;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

struct Removal {
    path: String,
    directory: PathBuf,
    token: u128,
    awaiting: std::collections::HashSet<u64>,
    restore_requested: bool,
    started: Duration,
}

#[derive(Resource, Default)]
pub(super) struct PendingRemoval(Option<Removal>);
impl PendingRemoval {
    pub(super) fn busy(&self) -> bool {
        self.0.is_some()
    }
}

pub(super) fn request_remove(
    mut requests: MessageReader<RosterRequest>,
    mut pending: ResMut<PendingRemoval>,
    catalog: Res<CharacterCatalog>,
    active: Res<ActiveCharacter>,
    settings: Res<NeraSettings>,
    job: Res<LibraryImport>,
    previews: Option<Res<super::previews::PreviewJobs>>,
    time: Res<Time<Real>>,
    mut instances: ResMut<CharacterInstances>,
    mut restore: MessageWriter<RestoreDefaultCharacter>,
    mut status: ResMut<CharacterImportStatus>,
) {
    for request in requests.read() {
        let RosterRequest::Remove { path } = request else {
            continue;
        };
        if !request.valid() || !catalog.0.iter().any(|entry| entry.path == *path) {
            continue;
        }
        if pending.busy()
            || job.0.is_some()
            || status.loading
            || previews.as_ref().is_some_and(|jobs| jobs.busy_for(path))
        {
            status.message = "Wait for the current character operation to finish.".into();
            continue;
        }
        let token = time.elapsed().as_nanos();
        for companion in &mut instances.companions {
            if let Some(pipe) = companion.child.stdin.as_mut() {
                let _ = writeln!(
                    pipe,
                    "RELEASE\t{}",
                    serde_json::to_string(&(path, token)).unwrap()
                );
            }
        }
        let restore_requested = active.asset_path == *path || settings.character_asset == *path;
        if restore_requested {
            restore.write(RestoreDefaultCharacter);
        }
        pending.0 = Some(Removal {
            path: path.clone(),
            directory: character_directory(),
            token,
            awaiting: instances
                .companions
                .iter()
                .map(|companion| companion.id)
                .collect(),
            restore_requested,
            started: time.elapsed(),
        });
        status.message = "Removing character...".into();
    }
}

pub(super) fn finish_remove(
    mut pending: ResMut<PendingRemoval>,
    active: Res<ActiveCharacter>,
    settings: Res<NeraSettings>,
    mut instances: ResMut<CharacterInstances>,
    acks: Option<Res<release::ReleaseAcks>>,
    time: Res<Time<Real>>,
    mut catalog: ResMut<CharacterCatalog>,
    mut status: ResMut<CharacterImportStatus>,
    mut previews: Option<ResMut<super::previews::PreviewJobs>>,
) {
    let Some(removal) = pending.0.as_mut() else {
        return;
    };
    if time.elapsed().saturating_sub(removal.started) > Duration::from_secs(60) {
        release::resume(&mut instances, &removal.path, removal.token);
        pending.0 = None;
        status.message = "Removal timed out; the character remains in the library.".into();
        return;
    }
    if let Some(acks) = &acks {
        for (id, active) in acks.drain(&removal.path, removal.token) {
            if removal.awaiting.remove(&id)
                && let Some(companion) = instances
                    .companions
                    .iter_mut()
                    .find(|companion| companion.id == id)
            {
                let name = catalog
                    .0
                    .iter()
                    .find(|entry| entry.path == active)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| {
                        saved_filename(&active)
                            .and_then(|file| model_metadata(&character_directory().join(file)).0)
                            .unwrap_or_else(|| "Imported Character".into())
                    });
                catalog.insert(&active, &name);
                companion.path = active;
                companion.name = name;
            }
        }
    }
    removal.awaiting.retain(|id| {
        instances
            .companions
            .iter()
            .any(|companion| companion.id == *id)
    });
    if status.loading || !removal.awaiting.is_empty() {
        return;
    }
    if active.asset_path == removal.path || settings.character_asset == removal.path {
        let reason = if removal.restore_requested {
            "Could not switch to Nera"
        } else {
            "Character became active"
        };
        status.message = format!("{reason}; removal cancelled.");
        release::resume(&mut instances, &removal.path, removal.token);
        pending.0 = None;
        return;
    }
    status.message = match remove_owned(&mut catalog, &removal.directory, &removal.path) {
        Ok(()) => {
            if let Some(previews) = previews.as_mut() {
                previews.forget(&removal.path);
            }
            crate::animation_manager::thumbnails::remove_cached_model(&removal.path);
            "Character removed. Import the original VRM to add it again.".into()
        }
        Err(reason) => format!("Could not remove character: {reason}"),
    };
    release::resume(&mut instances, &removal.path, removal.token);
    pending.0 = None;
}

fn remove_owned(
    catalog: &mut CharacterCatalog,
    directory: &Path,
    path: &str,
) -> Result<(), String> {
    let file = saved_filename(path).ok_or("Only imported characters can be removed")?;
    let Some(index) = catalog.0.iter().position(|entry| entry.path == path) else {
        return Ok(());
    };
    // An import is an immutable, content-addressed copy, never the original source.
    let original = directory.join(file);
    let staged = directory.join(format!("remove-{file}-{}.tmp", std::process::id()));
    let moved = match std::fs::rename(&original, &staged) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.to_string()),
    };
    let entry = catalog.0.remove(index);
    if let Err(error) = catalog.persist(directory) {
        catalog.0.insert(index, entry);
        if moved {
            let _ = std::fs::rename(&staged, &original);
        }
        return Err(error);
    }
    if moved && let Err(error) = std::fs::remove_file(&staged) {
        // Registry removal succeeded; a non-model temporary copy cannot be recovered as a character.
        warn!("Removed character's temporary copy could not be reclaimed: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_active_removal_waits_for_default_commit_and_clears_persisted_registry() {
        let directory =
            std::env::temp_dir().join(format!("nera-remove-commit-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = "characters://model-commit.vrm";
        let copy = directory.join(saved_filename(path).unwrap());
        std::fs::write(&copy, b"owned fixture").unwrap();
        let mut app = App::new();
        app.insert_resource(PendingRemoval(Some(Removal {
            path: path.into(),
            directory: directory.clone(),
            token: 1,
            awaiting: Default::default(),
            restore_requested: true,
            started: Duration::ZERO,
        })))
        .insert_resource(ActiveCharacter {
            asset_path: path.into(),
            migrated_from_vrm0: false,
        })
        .insert_resource(NeraSettings {
            character_asset: path.into(),
            ..default()
        })
        .insert_resource(CharacterImportStatus {
            loading: true,
            message: String::new(),
            ..default()
        })
        .init_resource::<CharacterInstances>()
        .init_resource::<Time<Real>>()
        .insert_resource(CharacterCatalog(vec![
            CharacterEntry {
                path: DEFAULT_CHARACTER.into(),
                name: "Nera".into(),
                facial_targets: 44,
                ..default()
            },
            CharacterEntry {
                path: path.into(),
                name: "Test".into(),
                facial_targets: 0,
                ..default()
            },
        ]))
        .add_systems(Update, finish_remove);
        app.update();
        assert!(copy.exists());
        assert!(app.world().resource::<PendingRemoval>().busy());
        // The existing replacement pipeline commits these owners, then finishes reveal.
        *app.world_mut().resource_mut::<ActiveCharacter>() = ActiveCharacter::default();
        app.world_mut()
            .resource_mut::<NeraSettings>()
            .character_asset
            .clear();
        app.world_mut()
            .resource_mut::<CharacterImportStatus>()
            .loading = false;
        app.update();
        assert!(!app.world().resource::<PendingRemoval>().busy());
        assert!(!copy.exists());
        assert_eq!(app.world().resource::<CharacterCatalog>().0.len(), 1);
        assert_eq!(
            app.world().resource::<ActiveCharacter>().asset_path,
            DEFAULT_CHARACTER
        );
        let saved: Vec<CharacterEntry> =
            serde_json::from_slice(&std::fs::read(directory.join("catalog.json")).unwrap())
                .unwrap();
        assert!(saved.is_empty());
        app.update();
        assert_eq!(app.world().resource::<CharacterCatalog>().0.len(), 1);
        std::fs::remove_file(directory.join("catalog.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn failed_catalog_save_rolls_back_registry_and_owned_copy() {
        let root =
            std::env::temp_dir().join(format!("nera-remove-rollback-{}", std::process::id()));
        std::fs::create_dir_all(root.join("catalog.json")).unwrap();
        let path = "characters://model-rollback.vrm";
        let file = root.join(saved_filename(path).unwrap());
        std::fs::write(&file, b"owned test copy").unwrap();
        let mut catalog = CharacterCatalog(vec![
            CharacterEntry {
                path: DEFAULT_CHARACTER.into(),
                name: "Nera".into(),
                facial_targets: 44,
                ..default()
            },
            CharacterEntry {
                path: path.into(),
                name: "Test".into(),
                facial_targets: 0,
                ..default()
            },
        ]);
        assert!(remove_owned(&mut catalog, &root, path).is_err());
        assert_eq!(catalog.0.len(), 2);
        assert_eq!(catalog.0[1].path, path);
        assert_eq!(std::fs::read(&file).unwrap(), b"owned test copy");
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_file(root.join(format!("catalog-{}.tmp", std::process::id()))).unwrap();
        std::fs::remove_dir(root.join("catalog.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn removal_preserves_source_and_builtin_and_allows_reimport() {
        let root = std::env::temp_dir().join(format!("nera-remove-{}", std::process::id()));
        let owned = root.join("owned");
        let source = crate::nera_asset_file(DEFAULT_CHARACTER);
        let before = std::fs::read(&source).unwrap();
        let imported = super::super::super::import_model(&source, &owned).unwrap();
        let mut catalog = CharacterCatalog(vec![
            CharacterEntry {
                path: DEFAULT_CHARACTER.into(),
                name: "Nera".into(),
                facial_targets: 44,
                ..default()
            },
            CharacterEntry {
                path: imported.path.clone(),
                name: "Test".into(),
                facial_targets: 44,
                ..default()
            },
        ]);
        catalog.persist(&owned).unwrap();
        assert!(remove_owned(&mut catalog, &owned, DEFAULT_CHARACTER).is_err());
        assert!(remove_owned(&mut catalog, &owned, "characters://../source.vrm").is_err());
        remove_owned(&mut catalog, &owned, &imported.path).unwrap();
        remove_owned(&mut catalog, &owned, &imported.path).unwrap();
        assert_eq!(catalog.0.len(), 1);
        assert_eq!(std::fs::read(&source).unwrap(), before);
        assert!(!owned.join(saved_filename(&imported.path).unwrap()).exists());
        let persisted: Vec<CharacterEntry> =
            serde_json::from_slice(&std::fs::read(owned.join("catalog.json")).unwrap()).unwrap();
        assert!(persisted.is_empty());
        let again = super::super::super::import_model(&source, &owned).unwrap();
        assert_eq!(again.path, imported.path);
        assert!(owned.join(saved_filename(&again.path).unwrap()).exists());
        for entry in std::fs::read_dir(&owned).unwrap().flatten() {
            std::fs::remove_file(entry.path()).unwrap();
        }
        std::fs::remove_dir(&owned).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn active_removal_requests_restore_and_never_removes_before_switch_finishes() {
        let mut app = App::new();
        let path = "characters://model-removal-test.vrm";
        app.init_resource::<PendingRemoval>()
            .init_resource::<LibraryImport>()
            .init_resource::<Time<Real>>()
            .init_resource::<NeraSettings>()
            .init_resource::<CharacterInstances>()
            .init_resource::<CharacterImportStatus>()
            .insert_resource(ActiveCharacter {
                asset_path: path.into(),
                migrated_from_vrm0: false,
            })
            .insert_resource(CharacterCatalog(vec![CharacterEntry {
                path: path.into(),
                name: "Test".into(),
                facial_targets: 0,
                ..default()
            }]))
            .add_message::<RosterRequest>()
            .add_message::<RestoreDefaultCharacter>()
            .add_systems(Update, request_remove);
        app.world_mut()
            .write_message(RosterRequest::Remove { path: path.into() });
        app.update();
        assert_eq!(
            app.world()
                .resource::<Messages<RestoreDefaultCharacter>>()
                .len(),
            1
        );
        assert!(app.world().resource::<PendingRemoval>().busy());
        app.world_mut()
            .resource_mut::<CharacterImportStatus>()
            .loading = true;
        app.add_systems(Update, finish_remove.after(request_remove));
        app.update();
        assert_eq!(app.world().resource::<CharacterCatalog>().0.len(), 1);
        app.world_mut()
            .resource_mut::<CharacterImportStatus>()
            .loading = false;
        app.update();
        assert!(!app.world().resource::<PendingRemoval>().busy());
        assert_eq!(app.world().resource::<CharacterCatalog>().0.len(), 1);
        assert_eq!(app.world().resource::<ActiveCharacter>().asset_path, path);
    }
}
