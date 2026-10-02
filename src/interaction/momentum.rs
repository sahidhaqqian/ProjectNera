use bevy::prelude::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct DragMomentum(VecDeque<(Instant, Vec2)>);

impl DragMomentum {
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
    pub(super) fn sample(&mut self, at: Instant, point: IVec2) {
        self.0.push_back((at, point.as_vec2()));
        while self.0.len() > 2
            && self.0.get(1).is_some_and(|(time, _)| {
                at.saturating_duration_since(*time) > Duration::from_millis(100)
            })
        {
            self.0.pop_front();
        }
        while self.0.len() > 32 {
            self.0.pop_front();
        }
    }
    pub(super) fn velocity(&self, scale: f32) -> Vec2 {
        let (Some((start, a)), Some((end, b))) = (self.0.front(), self.0.back()) else {
            return Vec2::ZERO;
        };
        let dt = end.saturating_duration_since(*start).as_secs_f32();
        if !(0.008..=0.18).contains(&dt) || !scale.is_finite() {
            return Vec2::ZERO;
        }
        let velocity = (*b - *a) / dt;
        if velocity.length() < 550.0 * scale.max(0.25) {
            return Vec2::ZERO;
        }
        velocity.clamp_length_max(2200.0 * scale.max(0.25))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_release_keeps_direction_but_slow_or_paused_drag_does_not_throw() {
        let now = Instant::now();
        let mut drag = DragMomentum::default();
        drag.sample(now, IVec2::ZERO);
        drag.sample(now + Duration::from_millis(50), IVec2::new(60, -40));
        let v = drag.velocity(1.0);
        assert!(v.x > 1000.0 && v.y < -700.0);
        drag.sample(now + Duration::from_millis(300), IVec2::new(60, -40));
        assert_eq!(drag.velocity(1.0), Vec2::ZERO);
        drag.clear();
        drag.sample(now, IVec2::ZERO);
        drag.sample(now + Duration::from_millis(50), IVec2::new(5, 0));
        assert_eq!(drag.velocity(1.0), Vec2::ZERO);
    }
    #[test]
    fn launch_is_bounded_and_threshold_scales_with_dpi() {
        let now = Instant::now();
        let mut drag = DragMomentum::default();
        drag.sample(now, IVec2::ZERO);
        drag.sample(now + Duration::from_millis(50), IVec2::new(600, 600));
        assert!(drag.velocity(1.0).length() <= 2200.1);
        drag.clear();
        drag.sample(now, IVec2::ZERO);
        drag.sample(now + Duration::from_millis(50), IVec2::new(40, 0));
        assert_ne!(drag.velocity(1.0), Vec2::ZERO);
        assert_eq!(drag.velocity(2.0), Vec2::ZERO);
    }
}
