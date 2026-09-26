//! Animation files used by Nera's active behaviors. Their paths are part of the runtime
//! contract, so the manager may preview them but must not rename, move, or
//! delete them.

pub(super) struct BaseAnimation {
    pub key: &'static str,
    pub label: &'static str,
    pub purpose: &'static str,
}

/// Display order for the dedicated "Animasi Dasar" section. This includes the
/// alarm clips and the radial-menu pose, but excludes unused legacy clips.
pub(super) const BASE_ANIMATIONS: &[BaseAnimation] = &[
    BaseAnimation {
        key: "animasi_basic/idle.vrma",
        label: "Base idle",
        purpose: "Nera's standing idle loop",
    },
    BaseAnimation {
        key: "animasi_basic/Walking.vrma",
        label: "Walking",
        purpose: "Automatic desktop walking loop",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_Enter.vrma",
        label: "Sit down",
        purpose: "Transition into sitting",
    },
    BaseAnimation {
        key: "animasi_basic/Sitting_Idle_Loop.vrma",
        label: "Sitting idle",
        purpose: "Sitting loop",
    },
    BaseAnimation {
        key: "animasi_basic/Floating.vrma",
        label: "Dragging / falling",
        purpose: "Loop used while dragging and falling",
    },
    BaseAnimation {
        key: "animasi_basic/Falling_To_Landing.vrma",
        label: "Landing",
        purpose: "Landing after a fall",
    },
    BaseAnimation {
        key: "animasi_basic/Hit_Head.vrma",
        label: "Head click",
        purpose: "Reaction to clicking Nera's head",
    },
    BaseAnimation {
        key: "animasi_basic/Hit_Chest.vrma",
        label: "Chest click",
        purpose: "Reaction to clicking Nera's chest",
    },
    BaseAnimation {
        key: "animasi_basic/ShakeHeadNo1.vrma",
        label: "Headpat",
        purpose: "Reaction to stroking Nera's head",
    },
    BaseAnimation {
        key: "animasi_basic/Death01.vrma",
        label: "Damage reaction",
        purpose: "Reaction to repeated hits",
    },
    BaseAnimation {
        key: "animasi_basic/Praying.vrma",
        label: "Radial menu pose",
        purpose: "Loop while the radial menu is open",
    },
    BaseAnimation {
        key: "animasi_basic/LookAround.vrma",
        label: "Idle: look around",
        purpose: "Automatic idle variation",
    },
    BaseAnimation {
        key: "animasi_basic/Relax.vrma",
        label: "Idle: relax",
        purpose: "Automatic idle variation",
    },
    BaseAnimation {
        key: "animasi_basic/Sleepy.vrma",
        label: "Idle: sleepy",
        purpose: "Automatic idle variation",
    },
    BaseAnimation {
        key: "animasi_basic/Blush.vrma",
        label: "Idle: blush",
        purpose: "Automatic idle variation",
    },
    BaseAnimation {
        key: "animasi_basic/jump_happy.vrma",
        label: "Alarm intro",
        purpose: "One-time jump when an alarm starts",
    },
    BaseAnimation {
        key: "animasi_basic/161_Waving.vrma",
        label: "Greeting / alarm wave",
        purpose: "One-time startup greeting and loop after the alarm intro",
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
        assert_eq!(BASE_ANIMATIONS.len(), 17);
        for (index, entry) in BASE_ANIMATIONS.iter().enumerate() {
            assert_eq!(base_order(entry.key), Some(index));
            assert!(!entry.purpose.is_empty());
        }
        assert!(!is_base_animation("Imported/idle.vrma"));
        assert!(!is_base_animation("animasi_basic/Sitting_Exit.vrma"));
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
