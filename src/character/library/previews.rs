//! Serial import/legacy-cache preparation; the browsing worker never loads a VRM.
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use std::collections::HashSet;

#[derive(Resource, Default)]
pub(super) struct PreviewJobs {
    attempted: HashSet<String>,
    job: Option<(String, Task<Result<(), String>>)>,
}

impl PreviewJobs {
    pub(super) fn busy_for(&self, path: &str) -> bool {
        self.job.as_ref().is_some_and(|(source, _)| source == path)
    }
    pub(super) fn forget(&mut self, path: &str) {
        self.attempted.remove(path);
    }
}

pub(super) fn install(app: &mut App) {
    if super::instance_id().is_none() {
        app.init_resource::<PreviewJobs>()
            .add_systems(Update, prepare.after(super::poll_library_import));
    }
}

fn prepare(
    catalog: Res<super::CharacterCatalog>,
    imports: Res<super::LibraryImport>,
    removal: Res<super::removal::PendingRemoval>,
    mut jobs: ResMut<PreviewJobs>,
    mut status: ResMut<super::super::CharacterImportStatus>,
) {
    if let Some((_, task)) = jobs.job.as_mut()
        && let Some(result) = block_on(poll_once(task))
    {
        let (path, _) = jobs.job.take().unwrap();
        match result {
            Ok(()) => status.thumbnail_revision = status.thumbnail_revision.wrapping_add(1),
            Err(reason) => warn!("Optional idle preview for {path}: {reason}"),
        }
    }
    if catalog.is_changed() {
        jobs.attempted
            .retain(|path| catalog.0.iter().any(|entry| &entry.path == path));
    }
    if jobs.job.is_some() || imports.0.is_some() || removal.busy() || status.loading {
        return;
    }
    if jobs.attempted.len() == catalog.0.len() {
        return;
    }
    let Some(path) = catalog
        .0
        .iter()
        .map(|entry| &entry.path)
        .find(|path| !jobs.attempted.contains(*path))
        .cloned()
    else {
        return;
    };
    jobs.attempted.insert(path.clone());
    let source = path.clone();
    jobs.job = Some((
        path,
        IoTaskPool::get()
            .spawn(async move { crate::animation_manager::thumbnails::prepare(&source) }),
    ));
}
