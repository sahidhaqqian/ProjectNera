//! AutoDance selection is subordinate to PetBehavior and the animation owner.
use crate::{
    animation::{LibraryPlayback, NeraAnimationState},
    dance_pool::DancePool,
    rng::NeraRng,
};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct AutoDance {
    pool: DancePool,
    last: Option<String>,
    pending: Option<String>,
    invalid: HashSet<String>,
    empty_logged: bool,
}

impl AutoDance {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(super) fn reject(&mut self, path: &str) {
        if self.pending.as_deref() == Some(path) {
            self.invalid
                .insert(path.trim_start_matches("animations/").into());
            self.pending = None;
        }
    }
    fn choose(&self, pool: &DancePool, rng: &mut NeraRng) -> Option<String> {
        let valid: Vec<_> = pool
            .keys
            .iter()
            .filter(|key| !self.invalid.contains(*key))
            .collect();
        let fresh: Vec<_> = valid
            .iter()
            .copied()
            .filter(|key| Some(key.as_str()) != self.last.as_deref())
            .collect();
        let candidates = if fresh.is_empty() { &valid } else { &fresh };
        rng.pick_index(candidates.len())
            .map(|index| candidates[index].to_string())
    }
    /// `allowed` already includes native placement, input, presentation, loading,
    /// cloud and music gates. Return true only if an actual clip owns the pose.
    pub(super) fn tick(
        &mut self,
        allowed: bool,
        pool: &DancePool,
        animation: &mut NeraAnimationState,
        library: &mut LibraryPlayback,
        rng: &mut NeraRng,
    ) -> bool {
        if self.pool != *pool {
            self.pool = pool.clone();
            self.invalid.clear();
            self.empty_logged = false;
            // An edited pool releases its old request/clip, never a manual clip.
            self.pending = None;
            library.cancel_auto_dance();
            animation.cancel_auto_dance();
        }
        if !allowed || !animation.can_auto_dance() || library.manual_pending() {
            self.pending = None;
            library.cancel_auto_dance();
            animation.cancel_auto_dance();
            return false;
        }
        if animation.auto_dance_active() {
            self.pending = None;
            return true;
        }
        if self
            .pending
            .as_deref()
            .is_some_and(|path| library.auto_dance_pending(path))
        {
            return false;
        }
        self.pending = None;
        if let Some(key) = self.choose(pool, rng) {
            let path = format!("animations/{key}");
            library.request_auto_dance(path.clone());
            self.last = Some(key);
            self.pending = Some(path);
        } else if !self.empty_logged {
            bevy::log::info!("AutoDance waiting: Dance Pool is empty or has no playable clips.");
            self.empty_logged = true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{BasePose, TemporaryAction, VisualState};
    use std::time::Duration;
    fn pool() -> DancePool {
        DancePool {
            keys: vec!["Bookmarks/a.vrma".into(), "Bookmarks/b.vrma".into()],
        }
    }
    #[test]
    fn empty_pool_never_interrupts_or_requests_playback() {
        let mut runtime = AutoDance::default();
        let mut state = NeraAnimationState::default();
        let mut library = LibraryPlayback::default();
        assert!(!runtime.tick(
            true,
            &DancePool::default(),
            &mut state,
            &mut library,
            &mut NeraRng::from_entropy()
        ));
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(!library.auto_dance_pending("animations/a.vrma"));
    }
    #[test]
    fn no_repeat_and_missing_clip_skipped_with_bounded_retry() {
        let mut runtime = AutoDance::default();
        let pool = pool();
        let mut rng = NeraRng::from_entropy();
        let first = runtime.choose(&pool, &mut rng).unwrap();
        runtime.last = Some(first.clone());
        assert_ne!(runtime.choose(&pool, &mut rng).unwrap(), first);
        runtime.invalid.insert(first.clone());
        runtime
            .invalid
            .insert(runtime.choose(&pool, &mut rng).unwrap());
        assert!(runtime.choose(&pool, &mut rng).is_none());
        runtime.reset();
        assert!(runtime.choose(&pool, &mut rng).is_some());
    }
    #[test]
    fn stop_block_and_character_replacement_release_only_autonomous_owner() {
        let mut runtime = AutoDance::default();
        let mut state = NeraAnimationState::default();
        let mut library = LibraryPlayback::default();
        let mut rng = NeraRng::from_entropy();
        let pool = pool();
        runtime.tick(true, &pool, &mut state, &mut library, &mut rng);
        assert!(runtime.pending.is_some());
        let entity = bevy::prelude::Entity::from_raw_u32(17).unwrap();
        assert!(state.request_auto_dance(entity, Duration::from_secs(1)));
        assert!(runtime.tick(true, &pool, &mut state, &mut library, &mut rng));
        runtime.tick(false, &pool, &mut state, &mut library, &mut rng);
        assert_eq!(state.visual_state(), VisualState::Idle);
        state.request_action_for_test(
            TemporaryAction::LibraryAnimation(entity),
            Duration::from_secs(3),
        );
        runtime.tick(false, &pool, &mut state, &mut library, &mut rng);
        assert_eq!(state.visual_state(), VisualState::LibraryAnimation(entity));
        runtime.reset();
        assert!(runtime.pending.is_none());
    }
    #[test]
    fn one_shot_finishes_then_requests_next_and_higher_owners_never_resume_a_tail() {
        let mut state = NeraAnimationState::default();
        let entity = bevy::prelude::Entity::from_raw_u32(18).unwrap();
        assert!(state.request_auto_dance(entity, Duration::from_secs(1)));
        state.advance(Duration::from_secs(1));
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(state.request_auto_dance(entity, Duration::from_secs(1)));
        for action in [
            TemporaryAction::Floating,
            TemporaryAction::AlarmWave,
            TemporaryAction::HeadStroke,
        ] {
            if !state.auto_dance_active() {
                assert!(state.request_auto_dance(entity, Duration::from_secs(1)));
            }
            state.request_action_for_test(action, Duration::from_secs(1));
            assert!(!state.auto_dance_active());
            assert!(!state.request_auto_dance(entity, Duration::from_secs(1)));
            state.advance(Duration::from_secs(1));
        }
        state.enter_cloud_seat(crate::animation::SittingPose::Sitting2);
        assert!(!state.request_auto_dance(entity, Duration::from_secs(1)));
        assert_eq!(state.base_pose(), BasePose::Sitting);
    }
    #[test]
    fn character_replacement_clears_old_dance_clip_and_preserves_both_body_modes_and_pool() {
        for mode in [
            crate::avatar::body_mode::AvatarBodyMode::Normal,
            crate::avatar::body_mode::AvatarBodyMode::Chibi,
        ] {
            let mut world = bevy::prelude::World::new();
            world.insert_resource(mode);
            world.insert_resource(pool());
            world.init_resource::<NeraAnimationState>();
            world.init_resource::<crate::pet_behavior::PetBehavior>();
            let entity = world.spawn_empty().id();
            assert!(
                world
                    .resource_mut::<NeraAnimationState>()
                    .request_auto_dance(entity, Duration::from_secs(1))
            );
            crate::animation::reset_for_character_change(&mut world);
            assert!(!world.resource::<NeraAnimationState>().auto_dance_active());
            assert_eq!(
                *world.resource::<crate::avatar::body_mode::AvatarBodyMode>(),
                mode
            );
            assert_eq!(*world.resource::<DancePool>(), pool());
        }
    }
}
