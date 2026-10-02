//! Semantic facial presets shared by the manager UI and the live VRM driver.
//! Only expression names present on the active avatar are emitted.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum NeraExpressionPreset {
    #[default]
    Default,
    Joy,
    Sad,
    Angry,
    Embarrassed,
    Timid,
    Surprised,
    Exhausted,
}

impl NeraExpressionPreset {
    pub(crate) const ALL: [Self; 8] = [
        Self::Default,
        Self::Joy,
        Self::Sad,
        Self::Angry,
        Self::Embarrassed,
        Self::Timid,
        Self::Surprised,
        Self::Exhausted,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Joy => "Joy",
            Self::Sad => "Sad",
            Self::Angry => "Angry",
            Self::Embarrassed => "Embarrassed",
            Self::Timid => "Timid",
            Self::Surprised => "Surprised",
            Self::Exhausted => "Exhausted",
        }
    }

    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Joy => "joy",
            Self::Sad => "sad",
            Self::Angry => "angry",
            Self::Embarrassed => "embarrassed",
            Self::Timid => "timid",
            Self::Surprised => "surprised",
            Self::Exhausted => "exhausted",
        }
    }

    pub(crate) fn from_wire(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.wire() == value)
    }
}

/// A matched custom expression takes precedence. Otherwise use VRM standard
/// emotions, with gentle combinations when there is no exact standard emotion.
/// Manual previews may control eyelids and mouth; clearing the preview restores
/// automatic blink, mouth and gaze ownership.
pub(crate) fn preset_weights<'a>(
    preset: NeraExpressionPreset,
    intensity: f32,
    available: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, f32)> {
    let available: Vec<_> = available.into_iter().collect();
    let find = |names: &[&str]| -> Option<String> {
        names.iter().find_map(|name| {
            available
                .iter()
                .find(|actual| actual.eq_ignore_ascii_case(name))
                .map(|actual| (*actual).to_owned())
        })
    };
    let mut result = Vec::new();
    let mut add = |names: &[&str], weight: f32| {
        if let Some(name) = find(names) {
            if !result.iter().any(|(existing, _)| existing == &name) {
                result.push((name, weight));
            }
        }
    };
    let custom = match preset {
        NeraExpressionPreset::Default => &[][..],
        NeraExpressionPreset::Joy => &["joy"][..],
        NeraExpressionPreset::Sad => &["sorrow"][..],
        NeraExpressionPreset::Angry => &["anger"][..],
        NeraExpressionPreset::Embarrassed => &["embarrassed", "blush", "shy"][..],
        NeraExpressionPreset::Timid => &["timid", "shy"][..],
        NeraExpressionPreset::Surprised => &["surprise"][..],
        NeraExpressionPreset::Exhausted => &["exhausted", "sleepy", "tired"][..],
    };
    if !custom.is_empty() && find(custom).is_some() {
        add(custom, 1.0);
    } else {
        match preset {
            NeraExpressionPreset::Default => {}
            NeraExpressionPreset::Joy => add(&["happy"], 1.0),
            NeraExpressionPreset::Sad => add(&["sad"], 1.0),
            NeraExpressionPreset::Angry => add(&["angry"], 1.0),
            NeraExpressionPreset::Surprised => add(&["surprised"], 1.0),
            NeraExpressionPreset::Embarrassed => {
                add(&["sad"], 0.65);
                add(&["aa"], 0.12);
            }
            NeraExpressionPreset::Timid => {
                add(&["happy"], 0.7);
                add(&["blink"], 1.0);
            }
            NeraExpressionPreset::Exhausted => {
                add(&["blink"], 1.0);
                add(&["oh", "ou"], 0.3);
                add(&["relaxed"], 0.2);
            }
        }
    }
    let intensity = if intensity.is_finite() {
        intensity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    result
        .into_iter()
        .filter_map(|(name, weight)| {
            let weight = weight * intensity;
            (weight > 0.0).then_some((name, weight))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_and_have_distinct_names() {
        let mut names = std::collections::HashSet::new();
        for preset in NeraExpressionPreset::ALL {
            assert!(names.insert(preset.name()));
            assert_eq!(NeraExpressionPreset::from_wire(preset.wire()), Some(preset));
        }
        assert_eq!(names.len(), 8);
    }

    #[test]
    fn missing_expressions_fall_back_without_emitting_unknown_names() {
        let available = ["happy", "relaxed"];
        assert_eq!(
            preset_weights(NeraExpressionPreset::Joy, 1.0, available),
            vec![("happy".into(), 1.0)]
        );
        assert!(preset_weights(NeraExpressionPreset::Angry, 1.0, available).is_empty());
        assert_eq!(
            preset_weights(NeraExpressionPreset::Embarrassed, 1.0, available),
            vec![]
        );
        assert_eq!(
            preset_weights(NeraExpressionPreset::Embarrassed, 1.0, ["blush", "happy"]),
            vec![("blush".into(), 1.0)]
        );
    }

    #[test]
    fn default_and_zero_intensity_clear_previous_weights() {
        for preset in NeraExpressionPreset::ALL {
            assert!(preset_weights(preset, 0.0, ["happy", "sad", "relaxed"]).is_empty());
        }
        assert!(preset_weights(NeraExpressionPreset::Default, 1.0, ["happy"]).is_empty());
    }

    #[test]
    fn intensity_clamps_and_switching_replaces_weights() {
        assert_eq!(
            preset_weights(NeraExpressionPreset::Joy, 2.0, ["happy"]),
            vec![("happy".into(), 1.0)]
        );
        assert!(preset_weights(NeraExpressionPreset::Joy, -1.0, ["happy"]).is_empty());
        assert_eq!(
            preset_weights(NeraExpressionPreset::Angry, 1.0, ["happy", "angry"]),
            vec![("angry".into(), 1.0)]
        );
    }
}
