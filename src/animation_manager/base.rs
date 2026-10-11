//! Animation files used by Nera's active behaviors. Their paths are part of the runtime
//! contract, so the manager may preview them but must not rename, move, or
//! delete them.

pub(super) struct BaseAnimation {
    pub key: &'static str,
    pub label: &'static str,
}

/// Display order for the dedicated "Animasi Dasar" section. This includes the
/// alarm clips and the radial-menu pose, but excludes unused legacy clips.
pub(super) const BASE_ANIMATIONS: &[BaseAnimation] = &[
    BaseAnimation {
        key: "animasi_basic/idle.vrma",
        label: "Base idle",
    },
    BaseAnimation {
        key: "animasi_basic/Walking.vrma",
        label: "Walking",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_Enter.vrma",
        label: "Sit down",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_Idle_Loop.vrma",
        label: "Sitting idle",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_2.vrma",
        label: "Alternative sitting position 2",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_4.vrma",
        label: "Alternative sitting position 4",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_Rubbing_Arm.vrma",
        label: "Sitting idle: rub arm",
    },
    BaseAnimation {
        key: "animasi_basic/Dodge_While_Sitting.vrma",
        label: "Sitting hit reaction",
    },
    BaseAnimation {
        key: "animasi_basic/Floating.vrma",
        label: "Dragging / falling",
    },
    BaseAnimation {
        key: "animasi_basic/Falling_To_Landing.vrma",
        label: "Landing",
    },
    BaseAnimation {
        key: "animasi_basic/Hit_Head.vrma",
        label: "Head click",
    },
    BaseAnimation {
        key: "animasi_basic/Hit_Chest.vrma",
        label: "Chest click",
    },
    BaseAnimation {
        key: "animasi_basic/ShakeHeadNo1.vrma",
        label: "Shake head",
    },
    BaseAnimation {
        key: "animasi_basic/Death01.vrma",
        label: "Damage reaction",
    },
    BaseAnimation {
        key: "animasi_basic/Praying.vrma",
        label: "Radial menu pose",
    },
    BaseAnimation {
        key: "animasi_basic/LookAround.vrma",
        label: "Idle: look around",
    },
    BaseAnimation {
        key: "animasi_basic/Relax.vrma",
        label: "Idle: relax",
    },
    BaseAnimation {
        key: "animasi_basic/Sleepy.vrma",
        label: "Idle: sleepy",
    },
    BaseAnimation {
        key: "animasi_basic/Sleeping_Idle.vrma",
        label: "Cloud rest: sleeping idle",
    },
    BaseAnimation {
        key: "animasi_basic/Blush.vrma",
        label: "Headpat / blush",
    },
    BaseAnimation {
        key: "animasi_basic/Catwalk_Idle_Twist_R.vrma",
        label: "Idle: catwalk twist",
    },
    BaseAnimation {
        key: "animasi_basic/Bored.vrma",
        label: "Idle: bored",
    },
    BaseAnimation {
        key: "animasi_basic/Angry_Gesture.vrma",
        label: "Idle: angry gesture",
    },
    BaseAnimation {
        key: "animasi_basic/jump_happy.vrma",
        label: "Alarm intro",
    },
    BaseAnimation {
        key: "animasi_basic/161_Waving.vrma",
        label: "Greeting / alarm wave",
    },
    BaseAnimation {
        key: "animasi_basic/wave1.vrma",
        label: "Hand touch: wave 1",
    },
    BaseAnimation {
        key: "animasi_basic/wave2.vrma",
        label: "Hand touch: wave 2",
    },
    BaseAnimation {
        key: "animasi_basic/salute.vrma",
        label: "Hand touch: salute",
    },
    BaseAnimation {
        key: "animasi_basic/thankful.vrma",
        label: "Hand touch: thankful",
    },
    BaseAnimation {
        key: "animasi_basic/Dismissive Gesture.vrma",
        label: "Hand touch: dismissive gesture",
    },
    BaseAnimation {
        key: "animasi_basic/Shrugging.vrma",
        label: "Hand touch: shrugging",
    },
    BaseAnimation {
        key: "animasi_basic/Arm Stretching.vrma",
        label: "Arm touch: stretching",
    },
    BaseAnimation {
        key: "animasi_basic/Shoulder Rubbing.vrma",
        label: "Arm touch: shoulder rubbing",
    },
    BaseAnimation {
        key: "animasi_basic/Waving.vrma",
        label: "Screensaver: greeting",
    },
    BaseAnimation {
        key: "animasi_basic/Breathing_Idle.vrma",
        label: "Screensaver: breathing idle",
    },
    BaseAnimation {
        key: "animasi_basic/Holding_Idle.vrma",
        label: "Screensaver: holding idle",
    },
    BaseAnimation {
        key: "animasi_basic/Dwarf_Idle_4.vrma",
        label: "Screensaver: dwarf idle 4",
    },
    BaseAnimation {
        key: "animasi_basic/Standing_WBriefcase_Idle_2.vrma",
        label: "Screensaver: briefcase idle 2",
    },
    BaseAnimation {
        key: "animasi_basic/Dwarf_Idle.vrma",
        label: "Screensaver: dwarf idle",
    },
    BaseAnimation {
        key: "animasi_basic/Dwarf_Idle_3.vrma",
        label: "Screensaver: dwarf idle 3",
    },
    BaseAnimation {
        key: "animasi_basic/Offensive_Idle.vrma",
        label: "Screensaver: offensive idle",
    },
    BaseAnimation {
        key: "animasi_basic/Dwarf_Idle_2.vrma",
        label: "Screensaver: dwarf idle 2 / firefly",
    },
];

pub(super) fn is_base_animation(key: &str) -> bool {
    base_order(key).is_some()
}

pub(super) fn base_label(key: &str) -> Option<&'static str> {
    base_order(key).map(|index| BASE_ANIMATIONS[index].label)
}

pub(super) fn base_order(key: &str) -> Option<usize> {
    BASE_ANIMATIONS
        .iter()
        .position(|entry| entry.key.eq_ignore_ascii_case(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation_manager::registry::AnimationRegistry;
    use std::{
        fs,
        io::ErrorKind,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn screensaver_animations_exist_and_use_the_same_protected_base_registry() {
        for clip in crate::animation::screensaver::ScreensaverClip::ALL {
            let key = clip.path().strip_prefix("animations/").unwrap();
            assert!(is_base_animation(key), "{key}");
            let bytes =
                fs::read(PathBuf::from(crate::nera_asset_path()).join(clip.path())).unwrap();
            assert_eq!(&bytes[..4], b"glTF");
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nera-base-animation-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            fs::create_dir(path.join("assets")).unwrap();
            Self(path)
        }

        fn write_animation(&self, key: &str, bytes: &[u8]) {
            let path = self.0.join("assets").join(key);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn registry(&self) -> AnimationRegistry {
            AnimationRegistry::new(self.0.join("assets"), self.0.join("bookmarks.txt"))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn base_keys_are_unique() {
        assert_eq!(BASE_ANIMATIONS.len(), 42);
        for (index, entry) in BASE_ANIMATIONS.iter().enumerate() {
            assert_eq!(base_order(entry.key), Some(index));
            assert!(!entry.label.is_empty());
        }
        assert!(!is_base_animation("Imported/idle.vrma"));
        assert!(!is_base_animation("animasi_basic/Sitting_Exit.vrma"));
    }

    #[test]
    fn touch_runtime_copies_do_not_claim_or_move_the_users_bookmarks() {
        let directory = TestDirectory::new();
        let copies = [
            ("wave1.vrma", "Wave.vrma"),
            ("wave2.vrma", "Wave2.vrma"),
            ("salute.vrma", "Salute.vrma"),
            ("thankful.vrma", "156_Thankful.vrma"),
            ("Dismissive Gesture.vrma", "Dismissing_Gesture.vrma"),
            ("Shrugging.vrma", "145_Shrugging.vrma"),
            ("Arm Stretching.vrma", "Arm_Stretching.vrma"),
            ("Shoulder Rubbing.vrma", "Shoulder_Rubbing.vrma"),
        ];
        let metadata = copies
            .iter()
            .map(|(_, original)| format!("Bookmarks/{original}\tLibrary/{original}\n"))
            .collect::<String>();
        fs::write(directory.0.join("bookmarks.txt"), metadata).unwrap();
        for (copy, original) in copies {
            directory.write_animation(&format!("animasi_basic/{copy}"), b"runtime copy");
            directory.write_animation(&format!("Bookmarks/{original}"), b"user original");
        }
        let mut registry = directory.registry();
        for (copy, original) in copies {
            let key = format!("animasi_basic/{copy}");
            let original_key = format!("Bookmarks/{original}");
            assert!(is_base_animation(&key.to_ascii_uppercase()));
            assert!(!is_base_animation(&original_key));
            assert!(registry.bookmarks.contains(&original_key));
            assert_eq!(
                registry.toggle_bookmark(&key).unwrap_err().kind(),
                ErrorKind::PermissionDenied
            );
            assert_eq!(
                fs::read(registry.path(&key).unwrap()).unwrap(),
                b"runtime copy"
            );
            assert_eq!(
                fs::read(registry.path(&original_key).unwrap()).unwrap(),
                b"user original"
            );
        }
    }

    #[test]
    fn inactive_clips_are_ordinary_editable_library_files() {
        let directory = TestDirectory::new();
        let unused = [
            "Unused/Sitting_Exit.vrma",
            "Unused/Head_get_hit.vrma",
            "Unused/Sitting_Talking_Loop.vrma",
            "Unused/sitting.vrma",
            "Falling.vrma",
            "Clawatar_animations/161_Waving.vrma",
            "VRMA/Thinking.vrma",
            "VRMA/Angry.vrma",
            "VRMA/Sad.vrma",
            "VRMA/Surprised.vrma",
        ];
        for key in unused {
            directory.write_animation(key, b"unused clip");
        }
        let mut registry = directory.registry();
        for (index, key) in unused.iter().enumerate() {
            assert!(!is_base_animation(key));
            let bookmarked = registry.toggle_bookmark(key).unwrap();
            let restored = registry.toggle_bookmark(&bookmarked).unwrap();
            assert_eq!(restored, *key);
            let renamed = registry
                .rename(&restored, &format!("Unused_{index}"))
                .unwrap();
            registry.delete(&renamed).unwrap();
        }
        assert!(registry.entries.is_empty());
    }

    #[test]
    fn promoted_sleeping_idle_is_basic_and_stale_bookmark_metadata_cannot_move_it_back() {
        let directory = TestDirectory::new();
        let key = "animasi_basic/Sleeping_Idle.vrma";
        directory.write_animation(key, b"sleep clip");
        fs::write(
            directory.0.join("bookmarks.txt"),
            "Bookmarks/Sleeping_Idle.vrma\tMixamo-VRMA/Sleeping_Idle.vrma\n",
        )
        .unwrap();
        let mut registry = directory.registry();
        assert!(registry.bookmarks.is_empty());
        assert_eq!(base_label(key), Some("Cloud rest: sleeping idle"));
        assert_eq!(
            registry.toggle_bookmark(key).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            fs::read(registry.path(key).unwrap()).unwrap(),
            b"sleep clip"
        );
        assert!(
            !directory
                .0
                .join("assets/Bookmarks/Sleeping_Idle.vrma")
                .exists()
        );
    }

    #[test]
    fn registry_preserves_every_base_file_when_edits_are_requested() {
        let directory = TestDirectory::new();
        for entry in BASE_ANIMATIONS {
            directory.write_animation(entry.key, entry.key.as_bytes());
        }
        // Old bookmark metadata must not move a runtime asset during startup.
        let legacy_bookmarks = BASE_ANIMATIONS
            .iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(directory.0.join("bookmarks.txt"), legacy_bookmarks).unwrap();

        let mut registry = directory.registry();
        assert!(registry.bookmarks.is_empty());
        for entry in BASE_ANIMATIONS {
            let source = directory.0.join("assets").join(entry.key);
            assert_eq!(fs::read(&source).unwrap(), entry.key.as_bytes());
            assert_eq!(
                registry
                    .rename(entry.key, "Changed_By_Manager")
                    .unwrap_err()
                    .kind(),
                ErrorKind::PermissionDenied,
                "rename {}",
                entry.key
            );
            assert_eq!(fs::read(&source).unwrap(), entry.key.as_bytes());
            assert_eq!(
                registry.delete(entry.key).unwrap_err().kind(),
                ErrorKind::PermissionDenied,
                "delete {}",
                entry.key
            );
            assert_eq!(fs::read(&source).unwrap(), entry.key.as_bytes());
            assert_eq!(
                registry.toggle_bookmark(entry.key).unwrap_err().kind(),
                ErrorKind::PermissionDenied,
                "bookmark {}",
                entry.key
            );
            assert_eq!(fs::read(&source).unwrap(), entry.key.as_bytes());
            assert!(registry.path(entry.key).is_some());
        }
        assert!(registry.bookmarks.is_empty());
        assert!(!directory.0.join("assets/Bookmarks").exists());
        assert_eq!(registry.entries.len(), BASE_ANIMATIONS.len());
    }

    #[test]
    fn registry_lists_base_animations_first_then_sorts_the_library() {
        let directory = TestDirectory::new();
        for entry in BASE_ANIMATIONS.iter().rev() {
            directory.write_animation(entry.key, b"base");
        }
        for key in [
            "Library/Zebra.vrma",
            "Library/Alpha.vrma",
            "Imported/idle.vrma",
        ] {
            directory.write_animation(key, b"ordinary");
        }

        let registry = directory.registry();
        let keys: Vec<_> = registry
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        let mut expected: Vec<_> = BASE_ANIMATIONS.iter().map(|entry| entry.key).collect();
        expected.extend([
            "Library/Alpha.vrma",
            "Imported/idle.vrma",
            "Library/Zebra.vrma",
        ]);
        assert_eq!(keys, expected);
    }
}
