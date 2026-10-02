//! Serial import/legacy-cache preparation; the browsing worker never loads a VRM.
use bevy::prelude::*;
use std::{collections::HashSet, thread::JoinHandle};

#[derive(Resource, Default)]
pub(super) struct PreviewJobs {
    attempted: HashSet<String>,
    job: Option<(String, JoinHandle<Result<(), String>>)>,
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
    if jobs
        .job
        .as_ref()
        .is_some_and(|(_, task)| task.is_finished())
    {
        let (path, task) = jobs.job.take().unwrap();
        let result = task
            .join()
            .unwrap_or_else(|_| Err("Thumbnail worker failed".into()));
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
    match spawn_job(move || crate::animation_manager::thumbnails::prepare(&source)) {
        Ok(task) => jobs.job = Some((path, task)),
        Err(error) => warn!("Optional idle preview for {path}: {error}"),
    }
}

fn spawn_job(
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> std::io::Result<JoinHandle<Result<(), String>>> {
    // This build deliberately omits Bevy's multi_threaded feature. Its
    // IoTaskPool::spawn eagerly polls on the caller, so the native preview
    // process's bounded wait must run on a real thread, never the UI thread.
    std::thread::Builder::new()
        .name("nera-character-preview".into())
        .spawn(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn preview_process_wait_cannot_block_the_ui_thread() {
        let caller = std::thread::current().id();
        let (started, start_notice) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel();
        let task = spawn_job(move || {
            started.send(std::thread::current().id()).unwrap();
            wait_for_release
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_ne!(
            start_notice.recv_timeout(Duration::from_secs(5)).unwrap(),
            caller
        );
        assert!(!task.is_finished());
        release.send(()).unwrap();
        task.join().unwrap().unwrap();
    }
}
