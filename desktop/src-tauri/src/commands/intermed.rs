use std::path::{Path, PathBuf};

use mc_core::agent::tools::IntermedConfig;
use mc_core::paths::{exe_dir, local_data_dir};

pub(super) fn bundled_intermed_config(resource_dir: Option<&Path>) -> Option<IntermedConfig> {
    let binary_name = if cfg!(windows) {
        "intermed.exe"
    } else {
        "intermed"
    };
    let mut candidates = Vec::new();

    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("KOBEMC_INTERMED_BIN") {
        candidates.push(PathBuf::from(path));
    }
    #[cfg(debug_assertions)]
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/intermed")
            .join(binary_name),
    );

    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join("intermed").join(binary_name));
    }
    candidates.push(exe_dir().join("intermed").join(binary_name));
    #[cfg(target_os = "macos")]
    candidates.push(exe_dir().join("../Resources/intermed").join(binary_name));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|binary| IntermedConfig {
            binary,
            cache_dir: local_data_dir().join("intermed"),
        })
}
