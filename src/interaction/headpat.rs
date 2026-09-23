//! Button-free head-stroke recognition and its independent cooldown.
use super::pointer::nera_pointer_position;
use super::{BodyPart, HoveredBodyPart, NeraHeadPatted};
use crate::{environment::DesktopEnvironment, platform};
use bevy::prelude::*;
use std::time::{Duration, Instant};

// -----------------------------------------------------------------------------
// Phase 5.4 head-stroke gesture tuning
// -----------------------------------------------------------------------------

/// "Elus kepala" accepts deliberate back-and-forth motion on either axis.
/// No mouse button is required, so it never competes with click-vs-drag.
const HEAD_STROKE_WINDOW: Duration = Duration::from_millis(900);
const HEAD_STROKE_COOLDOWN: Duration = Duration::from_millis(1200);
const HEAD_STROKE_MIN_TRAVEL_AT_96_DPI: f32 = 62.0;
const HEAD_STROKE_MIN_SEGMENT_AT_96_DPI: f32 = 10.0;
const HEAD_STROKE_JITTER_AT_96_DPI: f32 = 2.0;
const HEAD_STROKE_MIN_REVERSALS: u8 = 1;
#[derive(Resource, Default)]
pub(crate) struct HeadPatGesture {
    pub(crate) started_at: Option<Instant>,
    pub(crate) last_cursor: Option<IVec2>,
    pub(crate) total_travel: f32,

    pub(crate) x_direction: i8,
    pub(crate) y_direction: i8,
    pub(crate) x_segment: i32,
    pub(crate) y_segment: i32,
    pub(crate) x_reversals: u8,
    pub(crate) y_reversals: u8,

    pub(crate) cooldown_until: Option<Instant>,
}

impl HeadPatGesture {
    pub(crate) fn reset_stroke(&mut self) {
        self.started_at = None;
        self.last_cursor = None;
        self.total_travel = 0.0;
        self.x_direction = 0;
        self.y_direction = 0;
        self.x_segment = 0;
        self.y_segment = 0;
        self.x_reversals = 0;
        self.y_reversals = 0;
    }

    fn begin_stroke(&mut self, cursor: IVec2, now: Instant) {
        self.reset_stroke();
        self.started_at = Some(now);
        self.last_cursor = Some(cursor);
    }
}
#[cfg(target_os = "windows")]
pub(crate) fn detect_head_pat(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredBodyPart>,
    environment: Res<DesktopEnvironment>,
    mut gesture: ResMut<HeadPatGesture>,
    mut head_patted: MessageWriter<NeraHeadPatted>,
) {
    // Click/drag/right-click keep their own gesture ownership.
    if mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right) {
        gesture.reset_stroke();
        return;
    }

    if hovered.part != Some(BodyPart::Head) {
        gesture.reset_stroke();
        return;
    }

    let Some(point) = platform::global_cursor_position() else {
        gesture.reset_stroke();
        return;
    };

    let cursor = IVec2::new(point.x, point.y);
    let now = Instant::now();

    if gesture
        .cooldown_until
        .map(|until| now < until)
        .unwrap_or(false)
    {
        gesture.last_cursor = Some(cursor);
        return;
    }

    if gesture
        .started_at
        .map(|started| now.duration_since(started) > HEAD_STROKE_WINDOW)
        .unwrap_or(true)
    {
        gesture.begin_stroke(cursor, now);
        return;
    }

    let Some(previous) = gesture.last_cursor else {
        gesture.begin_stroke(cursor, now);
        return;
    };

    let delta = cursor - previous;
    gesture.last_cursor = Some(cursor);

    let scale = environment.nera_scale_factor.max(0.25) as f32;
    let jitter = (HEAD_STROKE_JITTER_AT_96_DPI * scale).round().max(1.0) as i32;
    let segment_min = (HEAD_STROKE_MIN_SEGMENT_AT_96_DPI * scale).round().max(1.0) as i32;
    let min_travel = HEAD_STROKE_MIN_TRAVEL_AT_96_DPI * scale;

    let delta_f = Vec2::new(delta.x as f32, delta.y as f32);
    let frame_travel = delta_f.length();

    if frame_travel < jitter as f32 {
        return;
    }

    gesture.total_travel += frame_travel;

    // Track reversal independently on X and Y so horizontal, vertical, and
    // natural diagonal/circular petting gestures all work.
    if delta.x.abs() >= jitter {
        let direction = delta.x.signum() as i8;

        if gesture.x_direction == 0 || gesture.x_direction == direction {
            gesture.x_direction = direction;
            gesture.x_segment += delta.x.abs();
        } else {
            if gesture.x_segment >= segment_min {
                gesture.x_reversals = gesture.x_reversals.saturating_add(1);
            }
            gesture.x_direction = direction;
            gesture.x_segment = delta.x.abs();
        }
    }

    if delta.y.abs() >= jitter {
        let direction = delta.y.signum() as i8;

        if gesture.y_direction == 0 || gesture.y_direction == direction {
            gesture.y_direction = direction;
            gesture.y_segment += delta.y.abs();
        } else {
            if gesture.y_segment >= segment_min {
                gesture.y_reversals = gesture.y_reversals.saturating_add(1);
            }
            gesture.y_direction = direction;
            gesture.y_segment = delta.y.abs();
        }
    }

    let reversals = gesture.x_reversals.saturating_add(gesture.y_reversals);

    if gesture.total_travel < min_travel || reversals < HEAD_STROKE_MIN_REVERSALS {
        return;
    }

    head_patted.write(NeraHeadPatted {
        pointer: nera_pointer_position(cursor, &environment),
    });

    info!(
        "Nera head-stroke recognized: travel={:.1}px x_rev={} y_rev={}",
        gesture.total_travel, gesture.x_reversals, gesture.y_reversals,
    );

    gesture.cooldown_until = Some(now + HEAD_STROKE_COOLDOWN);
    gesture.reset_stroke();
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn button_capture_cancels_head_stroke_without_clearing_cooldown_or_emitting() {
        for button in [MouseButton::Left, MouseButton::Right] {
            let now = Instant::now();
            let cooldown = now + HEAD_STROKE_COOLDOWN;
            let mut mouse = ButtonInput::<MouseButton>::default();
            mouse.press(button);
            let mut app = App::new();
            app.insert_resource(mouse)
                .insert_resource(HoveredBodyPart {
                    part: Some(BodyPart::Head),
                })
                .init_resource::<DesktopEnvironment>()
                .insert_resource(HeadPatGesture {
                    started_at: Some(now),
                    last_cursor: Some(IVec2::ONE),
                    total_travel: 100.0,
                    x_reversals: 2,
                    cooldown_until: Some(cooldown),
                    ..default()
                })
                .add_message::<NeraHeadPatted>()
                .add_systems(Update, detect_head_pat);
            app.update();
            let gesture = app.world().resource::<HeadPatGesture>();
            assert!(gesture.started_at.is_none());
            assert!(gesture.last_cursor.is_none());
            assert_eq!(gesture.total_travel, 0.0);
            assert_eq!(gesture.x_reversals, 0);
            assert_eq!(gesture.cooldown_until, Some(cooldown));
            assert!(
                app.world()
                    .resource::<Messages<NeraHeadPatted>>()
                    .is_empty()
            );
        }
    }
}
