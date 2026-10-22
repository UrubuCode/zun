use std::process::Command;

#[test]
fn build_aot_writes_fallback_module_manifest_and_object() {
    let root = std::env::temp_dir().join(format!("zun-aot-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): int { return 7; }\n").expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(source.with_extension("zm").is_file());
    assert!(source.with_extension("zman").is_file());
    assert!(source.with_extension("o").is_file());
    assert!(source.with_extension("exe").is_file());
    let manifest = zun_loader::ModuleManifest::read_file(source.with_extension("zman"))
        .expect("read AOT manifest");
    assert_eq!(manifest.native_object.as_deref(), Some("main.o"));
    let executable = Command::new(source.with_extension("exe"))
        .output()
        .expect("run AOT executable");
    assert!(executable.status.success());
    assert_eq!(String::from_utf8_lossy(&executable.stdout).trim(), "7");

    for path in [
        source.clone(),
        source.with_extension("zm"),
        source.with_extension("zman"),
        source.with_extension("o"),
        source.with_extension("exe"),
    ] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_runs_a_floating_point_entry_point() {
    let root = std::env::temp_dir().join(format!("zun-aot-float-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): number { return 7.5; }\n").expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let executable = Command::new(source.with_extension("exe"))
        .output()
        .expect("run AOT executable");
    assert!(executable.status.success());
    assert_eq!(String::from_utf8_lossy(&executable.stdout).trim(), "7.5");

    for extension in ["ts", "zm", "zman", "o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_keeps_the_module_when_entrypoint_needs_tier0() {
    let root = std::env::temp_dir().join(format!("zun-aot-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): string { return \"managed\"; }\n")
        .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("method #1 is not JIT-eligible"));
    assert!(source.with_extension("zm").is_file());
    assert!(source.with_extension("zman").is_file());
    let manifest = zun_loader::ModuleManifest::read_file(source.with_extension("zman"))
        .expect("read fallback manifest");
    assert!(manifest.native_object.is_none());
    assert!(!source.with_extension("exe").exists());

    for extension in ["ts", "zm", "zman", "o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn trim_reports_removed_method_names() {
    let root = std::env::temp_dir().join(format!("zun-trim-report-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(
        &source,
        "function unused(): int { return 9; }\nfunction main(): int { return 7; }\n",
    )
    .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--trim"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("trim: removing method #1 unused"));

    for extension in ["ts", "zm", "zman"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}
