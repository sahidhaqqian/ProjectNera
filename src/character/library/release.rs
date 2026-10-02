//! Removal handshake checks each companion's actual model, not the parent's last selection.
use super::super::{ActiveCharacter, CharacterImportStatus};
use super::*;
use std::io::BufReader;

#[derive(Resource, Default)]
pub(crate) struct ReleaseGuard(pub(super) Option<(String, u128, bool)>);
impl ReleaseGuard {
    pub(crate) fn busy(&self) -> bool {
        self.0.is_some()
    }
}

#[derive(Resource)]
pub(super) struct ReleaseAcks {
    sender: mpsc::SyncSender<(u64, String, u128, String)>,
    receiver: Mutex<mpsc::Receiver<(u64, String, u128, String)>>,
}
impl Default for ReleaseAcks {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(16);
        Self {
            sender,
            receiver: Mutex::new(receiver),
        }
    }
}
impl ReleaseAcks {
    pub(super) fn listen(&self, child: &mut Child, id: u64) {
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(json) = line.strip_prefix("NERA_RELEASED\t")
                    && let Ok((path, token, active)) =
                        serde_json::from_str::<(String, u128, String)>(json)
                    && saved_filename(&path).is_some()
                    && (active == DEFAULT_CHARACTER || saved_filename(&active).is_some())
                    && sender.send((id, path, token, active)).is_err()
                {
                    break;
                }
            }
        });
    }
    pub(super) fn drain(&self, path: &str, token: u128) -> Vec<(u64, String)> {
        let mut released_models = Vec::new();
        if let Ok(receiver) = self.receiver.lock() {
            for (id, released, generation, active) in receiver.try_iter() {
                if released == path && generation == token && active != path {
                    released_models.push((id, active));
                }
            }
        }
        released_models
    }
}

pub(super) fn finish_release(
    mut guard: ResMut<ReleaseGuard>,
    active: Res<ActiveCharacter>,
    status: Res<CharacterImportStatus>,
    job: Res<LibraryImport>,
    mut settings: ResMut<NeraSettings>,
    mut catalog: ResMut<CharacterCatalog>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some((path, token, acknowledged)) = guard.0.as_mut() else {
        return;
    };
    if *acknowledged || status.loading || job.0.is_some() {
        return;
    }
    catalog.0.retain(|entry| entry.path != *path);
    if active.asset_path == *path || settings.character_asset == *path {
        settings.character_asset.clear();
        crate::settings::save_settings_before_exit(&mut commands);
        exit.write(AppExit::Success);
    } else {
        println!(
            "NERA_RELEASED\t{}",
            serde_json::to_string(&(path, token, &active.asset_path)).unwrap()
        );
    }
    // Keep imports/selections blocked until the owner completes or cancels removal.
    *acknowledged = true;
}

pub(super) fn resume(instances: &mut CharacterInstances, path: &str, token: u128) {
    for companion in &mut instances.companions {
        if let Some(pipe) = companion.child.stdin.as_mut() {
            let _ = writeln!(
                pipe,
                "RELEASE_DONE\t{}",
                serde_json::to_string(&(path, token)).unwrap()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_waits_for_load_then_uses_real_active_model_and_clears_saved_reference() {
        let path = "characters://model-release.vrm";
        let mut app = App::new();
        app.insert_resource(ReleaseGuard(Some((path.into(), 9, false))))
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
            })
            .init_resource::<LibraryImport>()
            .insert_resource(CharacterCatalog(vec![CharacterEntry {
                path: path.into(),
                name: "Test".into(),
                facial_targets: 0,
            }]))
            .add_message::<AppExit>()
            .add_systems(Update, finish_release);
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        assert_eq!(app.world().resource::<CharacterCatalog>().0.len(), 1);
        app.world_mut()
            .resource_mut::<CharacterImportStatus>()
            .loading = false;
        app.update();
        assert_eq!(app.world().resource::<Messages<AppExit>>().len(), 1);
        assert!(
            app.world()
                .resource::<NeraSettings>()
                .character_asset
                .is_empty()
        );
        assert!(app.world().resource::<CharacterCatalog>().0.is_empty());
        app.update();
        assert_eq!(app.world().resource::<Messages<AppExit>>().len(), 1);
        assert!(app.world().resource::<ReleaseGuard>().busy());
    }
    #[test]
    fn stale_or_other_model_ack_cannot_release_current_removal() {
        let acks = ReleaseAcks::default();
        acks.sender
            .send((
                1,
                "characters://model-target.vrm".into(),
                8,
                DEFAULT_CHARACTER.into(),
            ))
            .unwrap();
        acks.sender
            .send((
                1,
                "characters://model-other.vrm".into(),
                9,
                DEFAULT_CHARACTER.into(),
            ))
            .unwrap();
        acks.sender
            .send((
                2,
                "characters://model-target.vrm".into(),
                9,
                DEFAULT_CHARACTER.into(),
            ))
            .unwrap();
        assert_eq!(
            acks.drain("characters://model-target.vrm", 9),
            vec![(2, DEFAULT_CHARACTER.into())]
        );
        acks.sender
            .send((
                1,
                "characters://model-target.vrm".into(),
                9,
                DEFAULT_CHARACTER.into(),
            ))
            .unwrap();
        assert_eq!(
            acks.drain("characters://model-target.vrm", 9),
            vec![(1, DEFAULT_CHARACTER.into())]
        );
        assert!(acks.drain("characters://model-target.vrm", 9).is_empty());
    }
}
