use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoResultExt};
use crate::instance::{Instance, InstanceConfig};
use crate::paths::GamePaths;

use super::ChatToolError;

const MAX_COPY_FILES: u64 = 50_000;
const MAX_COPY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TRIAL_OPERATIONS: usize = 10;
const EXCLUDED_TOP_LEVEL: &[&str] = &[
    "saves",
    "backups",
    "screenshots",
    "logs",
    "crash-reports",
    "natives",
    ".diagnostic-natives",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticTrialOperation {
    SetMemory { memory_mb: u32 },
    SetModEnabled { file_name: String, enabled: bool },
    DeleteMod { file_name: String },
}

#[derive(Debug, Clone)]
pub struct DiagnosticSandboxSnapshot {
    pub paths: GamePaths,
    pub instance_id: String,
    pub copied_files: u64,
    pub copied_bytes: u64,
}

#[derive(Default)]
struct CopyBudget {
    files: u64,
    bytes: u64,
}

pub fn create_diagnostic_snapshot(
    source_paths: &GamePaths,
    instance_id: &str,
    snapshot_root: &Path,
) -> Result<DiagnosticSandboxSnapshot, ChatToolError> {
    if !crate::fs::is_safe_segment(instance_id) {
        return Err(ChatToolError::new("invalid bound instance id"));
    }
    let source = source_paths.version_dir(instance_id);
    let metadata = std::fs::symlink_metadata(&source).with_path(&source)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ChatToolError::new(
            "bound instance directory must be a regular directory",
        ));
    }

    std::fs::create_dir_all(snapshot_root).with_path(snapshot_root)?;
    let source_canonical = source.canonicalize().with_path(&source)?;
    let snapshot_canonical = snapshot_root.canonicalize().with_path(snapshot_root)?;
    if snapshot_canonical.starts_with(&source_canonical) {
        return Err(ChatToolError::new(
            "diagnostic snapshot must be outside the installed instance",
        ));
    }

    let paths = GamePaths::new(snapshot_root);
    let destination = paths.version_dir(instance_id);
    if destination.exists() {
        return Err(ChatToolError::new("diagnostic snapshot already exists"));
    }
    std::fs::create_dir_all(&destination).with_path(&destination)?;
    let mut budget = CopyBudget::default();
    copy_instance_tree(&source, &destination, true, &mut budget)?;
    sanitize_snapshot_config(&destination.join("instance.json"))?;

    Ok(DiagnosticSandboxSnapshot {
        paths,
        instance_id: instance_id.to_string(),
        copied_files: budget.files,
        copied_bytes: budget.bytes,
    })
}

pub fn clone_diagnostic_snapshot(
    baseline: &DiagnosticSandboxSnapshot,
    trial_root: &Path,
) -> Result<DiagnosticSandboxSnapshot, ChatToolError> {
    let source = baseline.paths.version_dir(&baseline.instance_id);
    std::fs::create_dir_all(trial_root).with_path(trial_root)?;
    let paths = GamePaths::new(trial_root);
    let destination = paths.version_dir(&baseline.instance_id);
    if destination.exists() {
        return Err(ChatToolError::new("diagnostic trial already exists"));
    }
    std::fs::create_dir_all(&destination).with_path(&destination)?;
    let mut budget = CopyBudget::default();
    copy_instance_tree(&source, &destination, true, &mut budget)?;
    Ok(DiagnosticSandboxSnapshot {
        paths,
        instance_id: baseline.instance_id.clone(),
        copied_files: budget.files,
        copied_bytes: budget.bytes,
    })
}

pub fn apply_diagnostic_operations(
    paths: &GamePaths,
    instance_id: &str,
    operations: &[DiagnosticTrialOperation],
) -> Result<(), ChatToolError> {
    if operations.len() > MAX_TRIAL_OPERATIONS {
        return Err(ChatToolError::new(format!(
            "diagnostic trial accepts at most {MAX_TRIAL_OPERATIONS} operations"
        )));
    }
    let instance = Instance::new(instance_id, paths.root());
    let mut seen_mods = HashSet::new();
    let mut saw_memory = false;
    for operation in operations {
        match operation {
            DiagnosticTrialOperation::SetMemory { memory_mb } => {
                if !(512..=32_768).contains(memory_mb) || saw_memory {
                    return Err(ChatToolError::new(
                        "diagnostic memory must be unique and between 512 and 32768 MB",
                    ));
                }
                saw_memory = true;
            }
            DiagnosticTrialOperation::SetModEnabled { file_name, .. }
            | DiagnosticTrialOperation::DeleteMod { file_name } => {
                validate_mod_file(&instance, file_name)?;
                let key = file_name.trim_end_matches(".disabled").to_ascii_lowercase();
                if !seen_mods.insert(key) {
                    return Err(ChatToolError::new(
                        "a diagnostic trial may change each mod only once",
                    ));
                }
            }
        }
    }

    for operation in operations {
        match operation {
            DiagnosticTrialOperation::SetMemory { memory_mb } => {
                let mut config = instance.load_config()?;
                config.memory_mb = *memory_mb;
                instance.save_config(&config)?;
            }
            DiagnosticTrialOperation::SetModEnabled { file_name, enabled } => {
                crate::instance::mods::set_mod_enabled(&instance, file_name, *enabled)?;
            }
            DiagnosticTrialOperation::DeleteMod { file_name } => {
                let target = existing_mod_path(&instance, file_name).ok_or_else(|| {
                    ChatToolError::new(format!("mod file not found: {file_name}"))
                })?;
                std::fs::remove_file(&target).with_path(&target)?;
            }
        }
    }
    Ok(())
}

pub fn cleanup_diagnostic_session(session_root: &Path) -> Result<(), ChatToolError> {
    match std::fs::remove_dir_all(session_root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::io(session_root, error).into()),
    }
}

fn copy_instance_tree(
    source: &Path,
    destination: &Path,
    top_level: bool,
    budget: &mut CopyBudget,
) -> Result<(), ChatToolError> {
    let mut entries = std::fs::read_dir(source)
        .with_path(source)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CoreError::io(source, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name_string = name.to_string_lossy();
        if top_level && EXCLUDED_TOP_LEVEL.contains(&name_string.as_ref()) {
            continue;
        }
        let source_path = entry.path();
        let metadata = std::fs::symlink_metadata(&source_path).with_path(&source_path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let destination_path = destination.join(&name);
        if metadata.is_dir() {
            std::fs::create_dir_all(&destination_path).with_path(&destination_path)?;
            copy_instance_tree(&source_path, &destination_path, false, budget)?;
        } else if metadata.is_file() {
            budget.files = budget.files.saturating_add(1);
            budget.bytes = budget.bytes.saturating_add(metadata.len());
            if budget.files > MAX_COPY_FILES || budget.bytes > MAX_COPY_BYTES {
                return Err(ChatToolError::new(
                    "installed instance exceeds the diagnostic snapshot budget",
                ));
            }
            std::fs::copy(&source_path, &destination_path).with_path(&source_path)?;
        }
    }
    Ok(())
}

fn sanitize_snapshot_config(path: &Path) -> Result<(), ChatToolError> {
    let mut config = InstanceConfig::load(path)?;
    config.server = None;
    config.fullscreen = false;
    config.width = Some(854);
    config.height = Some(480);
    config.save(path)?;
    Ok(())
}

fn validate_mod_file(instance: &Instance, file_name: &str) -> Result<(), ChatToolError> {
    if !crate::fs::is_safe_segment(file_name)
        || !(file_name.ends_with(".jar") || file_name.ends_with(".jar.disabled"))
    {
        return Err(ChatToolError::new(format!(
            "invalid diagnostic mod file: {file_name}"
        )));
    }
    if existing_mod_path(instance, file_name).is_none() {
        return Err(ChatToolError::new(format!(
            "mod file not found: {file_name}"
        )));
    }
    Ok(())
}

fn existing_mod_path(instance: &Instance, file_name: &str) -> Option<PathBuf> {
    let base = file_name.trim_end_matches(".disabled");
    [
        instance.mods_dir().join(file_name),
        instance.mods_dir().join(base),
        instance.mods_dir().join(format!("{base}.disabled")),
    ]
    .into_iter()
    .find(|path| path.is_file())
}
