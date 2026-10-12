//! Derived mounted geometry; CloudRuntime remains the only placement owner.
//! VRM loading supplies Bevy's DynamicSkinnedMeshBounds, so each Aabb follows
//! the evaluated joints. GPU-only upload retains that compact skin metadata;
//! full vertex buffers remain released by the existing memory policy.
use super::visual::CloudVisual;
use crate::{environment::DesktopEnvironment, platform::PhysicalRect};
use bevy::{camera::primitives::Aabb, prelude::*};

#[derive(Clone, Copy, Debug)]
pub(super) struct VisibleBounds {
    pub min: Vec2,
    pub max: Vec2,
}
impl VisibleBounds {
    pub fn translated(self, delta: Vec2) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }
}

fn axis_correction(min: f32, max: f32, start: f32, end: f32) -> f32 {
    if max - min > end - start {
        // Oversized custom models cannot fit; maximize the visible area evenly.
        (start + end - min - max) * 0.5
    } else if min < start {
        start - min
    } else if max > end {
        end - max
    } else {
        0.0
    }
}
pub(super) fn work_area_correction(bounds: VisibleBounds, work: PhysicalRect) -> Vec2 {
    if !bounds.min.is_finite() || !bounds.max.is_finite() || work.width() <= 0 || work.height() <= 0
    {
        return Vec2::ZERO;
    }
    Vec2::new(
        axis_correction(
            bounds.min.x,
            bounds.max.x,
            work.left as f32,
            work.right as f32,
        ),
        axis_correction(
            bounds.min.y,
            bounds.max.y,
            work.top as f32,
            work.bottom as f32,
        ),
    )
    .round()
}

pub(super) fn ride_correction(bounds: VisibleBounds, seat: Vec2, screen: PhysicalRect) -> Vec2 {
    if !seat.is_finite() || !bounds.max.is_finite() {
        return Vec2::ZERO;
    }
    work_area_correction(
        VisibleBounds {
            min: bounds.min,
            max: Vec2::new(bounds.max.x, seat.y.max(bounds.min.y)),
        },
        screen,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_mounted_bounds(
    avatar: Entity,
    cloud: Entity,
    current_seat: Vec2,
    environment: &DesktopEnvironment,
    camera: &Camera,
    view: &GlobalTransform,
    character_scale: f32,
    children: &Query<&Children>,
    meshes: &Query<(&Aabb, &GlobalTransform), With<Mesh3d>>,
    visuals: &Query<(&CloudVisual, &GlobalTransform)>,
) -> Option<VisibleBounds> {
    let rect = environment.nera_rect?;
    let origin = Vec2::new(rect.left as f32, rect.top as f32);
    let dpi = environment.nera_scale_factor.max(0.25) as f32;
    let cloud_transform = visuals.get(cloud).ok().and_then(|(visual, root)| {
        visual
            .full_size_transform(current_seat, environment, camera, view, character_scale)
            .map(|full| (root.affine().inverse(), full))
    });
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for (root, is_cloud) in [(avatar, false), (cloud, true)] {
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok((aabb, global)) = meshes.get(entity) {
                let center: Vec3 = aabb.center.into();
                let half: Vec3 = aabb.half_extents.into();
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        for z in [-1.0, 1.0] {
                            let mut point =
                                global.transform_point(center + half * Vec3::new(x, y, z));
                            if is_cloud && let Some((inverse, full)) = cloud_transform {
                                point = full.transform_point(inverse.transform_point3(point));
                            }
                            // Protect both extremes of visual-only bobbing; a
                            // stable logical seat must not oscillate at the edge.
                            let bob = if is_cloud {
                                0.006 * character_scale
                            } else {
                                0.0
                            };
                            for offset in [-bob, bob] {
                                if let Ok(local) =
                                    camera.world_to_viewport(view, point + Vec3::Y * offset)
                                {
                                    let screen = origin + local * dpi;
                                    min = min.min(screen);
                                    max = max.max(screen);
                                }
                            }
                        }
                    }
                }
            }
            if let Ok(descendants) = children.get(entity) {
                stack.extend(descendants.iter());
            }
        }
    }
    (min.is_finite() && max.is_finite()).then_some(VisibleBounds { min, max })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ride_can_cross_taskbar_and_animated_feet_never_raise_the_seat() {
        for origin in [Vec2::ZERO, Vec2::new(-1920.0, -1080.0)] {
            for dpi in [1.0, 1.25, 1.5, 2.0] {
                let screen = PhysicalRect {
                    left: origin.x as i32,
                    top: origin.y as i32,
                    right: origin.x as i32 + 1920,
                    bottom: origin.y as i32 + 1080,
                };
                let seat = origin + Vec2::new(960.0, 1050.0);
                for foot_depth in [40.0, 100.0, 200.0] {
                    let bounds = VisibleBounds {
                        min: seat - Vec2::new(130.0, 280.0) * dpi,
                        max: seat + Vec2::new(130.0, foot_depth) * dpi,
                    };
                    assert_eq!(ride_correction(bounds, seat, screen), Vec2::ZERO);
                    let outside = Vec2::new(1500.0, 300.0);
                    let moved = bounds.translated(outside);
                    let correction = ride_correction(moved, seat + outside, screen);
                    let corrected = moved.translated(correction);
                    assert!(corrected.max.x <= screen.right as f32 + 0.5);
                    assert!((seat + outside + correction).y <= screen.bottom as f32 + 0.5);
                    assert_eq!(
                        ride_correction(corrected, seat + outside + correction, screen),
                        Vec2::ZERO
                    );
                }
            }
        }
    }
    #[test]
    fn whole_mount_stays_inside_each_taskbar_edge_at_multiple_dpi_sizes() {
        for work in [
            PhysicalRect {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1000,
            },
            PhysicalRect {
                left: 80,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            PhysicalRect {
                left: 0,
                top: 80,
                right: 1920,
                bottom: 1080,
            },
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1840,
                bottom: 1080,
            },
        ] {
            for dpi in [1.0, 1.25, 1.5, 2.0] {
                for center in [
                    Vec2::new(work.left as f32 - 100.0, work.top as f32 - 100.0),
                    Vec2::new(work.right as f32 + 100.0, work.bottom as f32 + 100.0),
                ] {
                    let half = Vec2::new(130.0, 180.0) * dpi;
                    let original = VisibleBounds {
                        min: center - half,
                        max: center + half,
                    };
                    let corrected = original.translated(work_area_correction(original, work));
                    assert!(corrected.min.x >= work.left as f32 - 0.5);
                    assert!(corrected.min.y >= work.top as f32 - 0.5);
                    assert!(corrected.max.x <= work.right as f32 + 0.5);
                    assert!(corrected.max.y <= work.bottom as f32 + 0.5);
                    assert_eq!(work_area_correction(corrected, work), Vec2::ZERO);
                }
            }
        }
    }
    #[test]
    fn oversized_and_invalid_geometry_cannot_panic_or_oscillate() {
        let work = PhysicalRect {
            left: 0,
            top: 0,
            right: 500,
            bottom: 400,
        };
        let original = VisibleBounds {
            min: Vec2::splat(-500.0),
            max: Vec2::splat(600.0),
        };
        let corrected = original.translated(work_area_correction(original, work));
        assert_eq!(work_area_correction(corrected, work), Vec2::ZERO);
        assert_eq!(
            work_area_correction(
                VisibleBounds {
                    min: Vec2::NAN,
                    max: Vec2::ONE
                },
                work
            ),
            Vec2::ZERO
        );
    }
}
