//! Facial intent shared by desktop playback and basic-animation previews.
use super::NeraExpressionPreset as Face;
use crate::animation::{IdleMicroAction, VisualState};

pub(crate) fn for_clip(path: &str) -> Option<(Face, f32)> {
    let file = path
        .strip_prefix("animations/")
        .unwrap_or(path)
        .strip_prefix("animasi_basic/")?;
    Some(match file {
        "idle.vrma" => (Face::Default, 1.0),
        "Walking.vrma" => (Face::Joy, 0.2),
        "Sitting_Enter.vrma" | "Sitting_Idle_Loop.vrma" | "Sitting_2.vrma" => (Face::Timid, 0.3),
        "Sitting_4.vrma" => (Face::Joy, 0.3),
        "Sitting_Rubbing_Arm.vrma" => (Face::Embarrassed, 0.45),
        "Dodge_While_Sitting.vrma" | "Hit_Head.vrma" => (Face::Surprised, 0.8),
        "Floating.vrma" => (Face::Surprised, 0.7),
        "Falling_To_Landing.vrma" => (Face::Surprised, 0.5),
        "Hit_Chest.vrma" | "Angry_Gesture.vrma" => (Face::Angry, 0.9),
        "ShakeHeadNo1.vrma" => (Face::Sad, 0.5),
        "Death01.vrma" => (Face::Sad, 0.9),
        "Praying.vrma" => (Face::Timid, 0.4),
        "LookAround.vrma" => (Face::Surprised, 0.35),
        "Relax.vrma" => (Face::Timid, 0.35),
        "Sleepy.vrma" => (Face::Exhausted, 0.6),
        "Blush.vrma" => (Face::Embarrassed, 0.75),
        "Catwalk_Idle_Twist_R.vrma" => (Face::Joy, 0.45),
        "Bored.vrma" => (Face::Exhausted, 0.35),
        "jump_happy.vrma" => (Face::Joy, 0.85),
        "161_Waving.vrma" => (Face::Joy, 0.8),
        _ => return None,
    })
}

pub(crate) fn for_visual(visual: VisualState, library_path: Option<&str>) -> Option<(Face, f32)> {
    use VisualState as Body;
    let file = match visual {
        Body::HeadStroke => "Blush.vrma",
        Body::HitChest => "Hit_Chest.vrma",
        Body::HitHead => "Hit_Head.vrma",
        Body::Death => "Death01.vrma",
        Body::Praying => "Praying.vrma",
        Body::Idle => "idle.vrma",
        Body::Walking => "Walking.vrma",
        Body::Floating | Body::Falling => "Floating.vrma",
        Body::Landing => "Falling_To_Landing.vrma",
        Body::SittingEnter => "Sitting_Enter.vrma",
        Body::SittingLoop => "Sitting_Idle_Loop.vrma",
        Body::DodgeWhileSitting => "Dodge_While_Sitting.vrma",
        Body::StartupGreeting | Body::AlarmWave => "161_Waving.vrma",
        Body::AlarmJump => "jump_happy.vrma",
        Body::IdleMicro(action) => match action {
            IdleMicroAction::LookAround => "LookAround.vrma",
            IdleMicroAction::Relax => "Relax.vrma",
            IdleMicroAction::Sleepy => "Sleepy.vrma",
            IdleMicroAction::Blush => "Blush.vrma",
            IdleMicroAction::CatwalkIdleTwistR => "Catwalk_Idle_Twist_R.vrma",
            IdleMicroAction::Bored => "Bored.vrma",
            IdleMicroAction::AngryGesture => "Angry_Gesture.vrma",
            IdleMicroAction::SittingRubbingArm => "Sitting_Rubbing_Arm.vrma",
        },
        Body::LibraryAnimation(_) => return library_path.and_then(for_clip),
        _ => return None,
    };
    for_clip(&format!("animasi_basic/{file}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_bundled_basic_clip_has_a_face() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/animations/animasi_basic");
        let mut count = 0;
        for entry in std::fs::read_dir(directory).unwrap().flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            if file.ends_with(".vrma") {
                let (_, intensity) = for_clip(&format!("animasi_basic/{file}"))
                    .unwrap_or_else(|| panic!("No face for {file}"));
                assert!((0.0..=1.0).contains(&intensity));
                count += 1;
            }
        }
        assert_eq!(count, 24);
    }
    #[test]
    fn requested_pairs_and_unknown_clip_ownership() {
        assert_eq!(
            for_visual(VisualState::HeadStroke, None),
            Some((Face::Embarrassed, 0.75))
        );
        assert_eq!(
            for_clip("animations/animasi_basic/Angry_Gesture.vrma"),
            Some((Face::Angry, 0.9))
        );
        assert_eq!(
            for_clip("animasi_basic/Sleepy.vrma"),
            Some((Face::Exhausted, 0.6))
        );
        assert_eq!(for_clip("imported/Dance.vrma"), None);
    }
}
