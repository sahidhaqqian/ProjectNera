//! Packaged assets always win; release binaries never use a compiler-machine path.
use std::path::{Path, PathBuf};

fn choose(
    installed: PathBuf,
    development: PathBuf,
    debug: bool,
    installed_exists: bool,
    development_exists: bool,
) -> PathBuf {
    if debug && !installed_exists && development_exists {
        development
    } else {
        installed
    }
}

pub(crate) fn directory() -> PathBuf {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default()
        .join("assets");
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let installed_exists = installed.is_dir();
    let development_exists = cfg!(debug_assertions) && development.is_dir();
    choose(
        installed,
        development,
        cfg!(debug_assertions),
        installed_exists,
        development_exists,
    )
}

pub(crate) fn missing_core(directory: &Path) -> Vec<&'static str> {
    [
        "vrm/nera_256.vrm",
        "animations/animasi_basic/idle.vrma",
        "shaders/nera_shadow_only.wgsl",
    ]
    .into_iter()
    .filter(|path| !directory.join(path).is_file())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_never_uses_development_assets_even_when_the_package_is_incomplete() {
        for present in [false, true] {
            assert_eq!(
                choose(
                    "package/assets".into(),
                    "developer/assets".into(),
                    false,
                    present,
                    true
                ),
                PathBuf::from("package/assets")
            );
        }
    }
    #[test]
    fn packaged_assets_win_in_debug_and_development_is_only_a_debug_fallback() {
        assert_eq!(
            choose(
                "package/assets".into(),
                "developer/assets".into(),
                true,
                true,
                true
            ),
            PathBuf::from("package/assets")
        );
        assert_eq!(
            choose(
                "package/assets".into(),
                "developer/assets".into(),
                true,
                false,
                true
            ),
            PathBuf::from("developer/assets")
        );
        assert_eq!(
            choose(
                "package/assets".into(),
                "developer/assets".into(),
                true,
                false,
                false
            ),
            PathBuf::from("package/assets")
        );
    }
}
