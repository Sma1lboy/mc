use super::*;

#[test]
fn compatibility_report_status_follows_highest_severity() {
    let report = CompatibilityReport::from_issues(vec![
        CompatibilityIssue::new(
            "memory_below_recommendation",
            IssueSeverity::Warning,
            "Memory is below the recommendation",
        ),
        CompatibilityIssue::new(
            "duplicate_mod_id",
            IssueSeverity::Blocking,
            "Duplicate mod id detected",
        ),
    ]);

    assert_eq!(report.status, CompatibilityStatus::Blocked);
    assert_eq!(report.issues.len(), 2);
    assert_eq!(report.issues[0].code, "memory_below_recommendation");
}

fn write_diagnostic_instance(root: &std::path::Path) -> GamePaths {
    let paths = GamePaths::new(root);
    let base_dir = paths.version_dir("1.20.1");
    let instance_dir = paths.version_dir("forge-pack");
    std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
    std::fs::create_dir_all(instance_dir.join("logs")).unwrap();
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();
    std::fs::write(
        paths.version_json("forge-pack"),
        r#"{"id":"forge-pack","inheritsFrom":"1.20.1"}"#,
    )
    .unwrap();
    std::fs::write(
        instance_dir.join("instance.json"),
        r#"{"name":"Forge Pack","memory_mb":2048}"#,
    )
    .unwrap();

    let first = br#"{"schemaVersion":1,"id":"duplicate","version":"1.0.0","name":"Duplicate One"}"#;
    let second =
        br#"{"schemaVersion":1,"id":"duplicate","version":"2.0.0","name":"Duplicate Two"}"#;
    std::fs::write(
        instance_dir.join("mods/duplicate-one.jar"),
        zip_bytes(&[("fabric.mod.json", first)]),
    )
    .unwrap();
    std::fs::write(
        instance_dir.join("mods/duplicate-two.jar"),
        zip_bytes(&[("fabric.mod.json", second)]),
    )
    .unwrap();
    std::fs::write(
        instance_dir.join("logs/latest.log"),
        "Exception in thread main java.lang.OutOfMemoryError: Java heap space",
    )
    .unwrap();
    paths
}

fn write_sandbox_source(root: &std::path::Path) -> GamePaths {
    let paths = GamePaths::new(root);
    let instance_dir = paths.version_dir("test-pack");
    let loader_dir = paths.version_dir("fabric-loader");
    let vanilla_dir = paths.version_dir("1.20.1");
    std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
    std::fs::create_dir_all(instance_dir.join("config")).unwrap();
    std::fs::create_dir_all(instance_dir.join("saves/world")).unwrap();
    std::fs::create_dir_all(instance_dir.join("logs")).unwrap();
    std::fs::create_dir_all(&loader_dir).unwrap();
    std::fs::create_dir_all(&vanilla_dir).unwrap();
    std::fs::create_dir_all(paths.assets_dir()).unwrap();
    std::fs::create_dir_all(paths.libraries_dir()).unwrap();
    std::fs::write(
        paths.version_json("test-pack"),
        r#"{"id":"test-pack","inheritsFrom":"fabric-loader"}"#,
    )
    .unwrap();
    std::fs::write(
        paths.version_json("fabric-loader"),
        r#"{"id":"fabric-loader","inheritsFrom":"1.20.1"}"#,
    )
    .unwrap();
    std::fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();
    std::fs::write(instance_dir.join("instance.json"), r#"{"memory_mb":2048}"#).unwrap();
    std::fs::write(instance_dir.join("mods/keep.jar"), b"jar").unwrap();
    std::fs::write(instance_dir.join("mods/remove.jar"), b"jar").unwrap();
    std::fs::write(instance_dir.join("config/pack.cfg"), b"enabled=true").unwrap();
    std::fs::write(instance_dir.join("saves/world/level.dat"), b"world").unwrap();
    std::fs::write(instance_dir.join("logs/latest.log"), b"old log").unwrap();
    paths
}

#[test]
fn diagnostic_snapshot_copies_runtime_inputs_and_excludes_user_data() {
    let source_root = temp_dir("diagnostic-snapshot-source");
    let session_root = temp_dir("diagnostic-snapshot-session");
    let source = write_sandbox_source(&source_root);

    let snapshot = create_diagnostic_snapshot(&source, "test-pack", &session_root).unwrap();

    assert!(snapshot
        .paths
        .version_dir("test-pack")
        .join("mods/keep.jar")
        .is_file());
    assert!(snapshot
        .paths
        .version_dir("test-pack")
        .join("config/pack.cfg")
        .is_file());
    assert!(!snapshot
        .paths
        .version_dir("test-pack")
        .join("saves")
        .exists());
    assert!(!snapshot
        .paths
        .version_dir("test-pack")
        .join("logs")
        .exists());
    assert_eq!(
        std::fs::read(source.version_dir("test-pack").join("mods/keep.jar")).unwrap(),
        b"jar"
    );

    std::fs::remove_dir_all(source_root).unwrap();
    std::fs::remove_dir_all(session_root).unwrap();
}

#[cfg(unix)]
#[test]
fn diagnostic_snapshot_skips_source_symlinks() {
    use std::os::unix::fs::symlink;

    let source_root = temp_dir("diagnostic-snapshot-symlink-source");
    let session_root = temp_dir("diagnostic-snapshot-symlink-session");
    let outside = temp_dir("diagnostic-snapshot-symlink-outside");
    let source = write_sandbox_source(&source_root);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.jar"), b"secret").unwrap();
    symlink(
        outside.join("secret.jar"),
        source.version_dir("test-pack").join("mods/linked.jar"),
    )
    .unwrap();

    let snapshot = create_diagnostic_snapshot(&source, "test-pack", &session_root).unwrap();

    assert!(!snapshot
        .paths
        .version_dir("test-pack")
        .join("mods/linked.jar")
        .exists());
    std::fs::remove_dir_all(source_root).unwrap();
    std::fs::remove_dir_all(session_root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn deep_diagnosis_operations_change_only_the_snapshot() {
    let source_root = temp_dir("diagnostic-ops-source");
    let session_root = temp_dir("diagnostic-ops-session");
    let source = write_sandbox_source(&source_root);
    let snapshot = create_diagnostic_snapshot(&source, "test-pack", &session_root).unwrap();

    apply_diagnostic_operations(
        &snapshot.paths,
        "test-pack",
        &[
            DiagnosticTrialOperation::SetMemory { memory_mb: 4096 },
            DiagnosticTrialOperation::SetModEnabled {
                file_name: "keep.jar".into(),
                enabled: false,
            },
            DiagnosticTrialOperation::DeleteMod {
                file_name: "remove.jar".into(),
            },
        ],
    )
    .unwrap();

    let trial_dir = snapshot.paths.version_dir("test-pack");
    assert_eq!(
        crate::instance::InstanceConfig::load(&trial_dir.join("instance.json"))
            .unwrap()
            .memory_mb,
        4096
    );
    assert!(trial_dir.join("mods/keep.jar.disabled").is_file());
    assert!(!trial_dir.join("mods/remove.jar").exists());
    assert!(source
        .version_dir("test-pack")
        .join("mods/keep.jar")
        .is_file());
    assert!(source
        .version_dir("test-pack")
        .join("mods/remove.jar")
        .is_file());

    std::fs::remove_dir_all(source_root).unwrap();
    std::fs::remove_dir_all(session_root).unwrap();
}

#[test]
fn deep_diagnosis_operations_reject_paths_unknown_mods_and_large_plans() {
    let source_root = temp_dir("diagnostic-invalid-source");
    let session_root = temp_dir("diagnostic-invalid-session");
    let source = write_sandbox_source(&source_root);
    let snapshot = create_diagnostic_snapshot(&source, "test-pack", &session_root).unwrap();

    for bad in ["../keep.jar", "nested/keep.jar", "missing.jar"] {
        let result = apply_diagnostic_operations(
            &snapshot.paths,
            "test-pack",
            &[DiagnosticTrialOperation::DeleteMod {
                file_name: bad.into(),
            }],
        );
        assert!(result.is_err(), "{bad} must be rejected");
    }
    let excessive = (0..11)
        .map(|_| DiagnosticTrialOperation::SetMemory { memory_mb: 2048 })
        .collect::<Vec<_>>();
    assert!(apply_diagnostic_operations(&snapshot.paths, "test-pack", &excessive).is_err());

    std::fs::remove_dir_all(source_root).unwrap();
    std::fs::remove_dir_all(session_root).unwrap();
}

#[test]
fn diagnose_instance_reports_structural_and_log_issues() {
    let root = temp_dir("diagnose-instance");
    let paths = write_diagnostic_instance(&root);

    let output = diagnose_instance_with_total_memory(
        &paths,
        "forge-pack",
        DiagnoseInstanceArgs {
            include_log_tail: true,
        },
        16_384,
    )
    .unwrap();

    assert_eq!(output.instance.name, "Forge Pack");
    assert_eq!(output.instance.mc_version, "1.20.1");
    assert_eq!(output.instance.loader, "forge");
    assert_eq!(output.instance.mod_count, 2);
    assert_eq!(output.report.status, CompatibilityStatus::Blocked);
    let codes: Vec<_> = output
        .report
        .issues
        .iter()
        .map(|issue| issue.code.as_str())
        .collect();
    assert!(codes.contains(&"duplicate_mod_id"));
    assert!(codes.contains(&"mod_loader_mismatch"));
    assert!(codes.contains(&"memory_below_recommendation"));
    assert!(codes.contains(&"last_launch_crash"));
    assert!(output
        .log_tail
        .as_deref()
        .unwrap()
        .contains("OutOfMemoryError"));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn diagnose_instance_uses_logs_without_exposing_tail_by_default() {
    let root = temp_dir("diagnose-instance-hidden-log");
    let paths = write_diagnostic_instance(&root);

    let output = diagnose_instance_with_total_memory(
        &paths,
        "forge-pack",
        DiagnoseInstanceArgs {
            include_log_tail: false,
        },
        16_384,
    )
    .unwrap();

    assert!(output.log_tail.is_none());
    assert!(output
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "last_launch_crash"));

    std::fs::remove_dir_all(root).unwrap();
}
