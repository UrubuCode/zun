use std::process::Command;

#[test]
fn build_can_persist_a_valid_prelude_cache() {
    let root = std::env::temp_dir().join(format!("zun-prelude-cache-cli-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    let cache = root.join("prelude.zprc");
    std::fs::write(&source, "function main(): int { return abs(-7); }\n")
        .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--prelude-cache"])
        .arg(&cache)
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(cache.is_file());
    zun_compiler::read_prelude_cache(&cache).expect("cache must be readable");
    let module = zun_module::Module::read(
        &std::fs::read(source.with_extension("zm")).expect("read linked module"),
    )
    .expect("linked module must be valid");
    assert_eq!(module.tables.member_refs.len(), 1);
    let manifest = zun_loader::ModuleManifest::read_file(source.with_extension("zman"))
        .expect("read linked manifest");
    assert_eq!(manifest.dependencies, vec!["zun:prelude"]);

    for path in [source.clone(), source.with_extension("zm"), source.with_extension("zman"), cache] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_rejects_a_stale_prelude_cache_before_emitting_module() {
    let root = std::env::temp_dir().join(format!(
        "zun-prelude-cache-stale-cli-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    let cache = root.join("prelude.zprc");
    std::fs::write(&source, "function main(): int { return 7; }\n").expect("write source");
    zun_compiler::write_prelude_cache(&cache).expect("write valid cache");
    let mut bytes = std::fs::read(&cache).expect("read cache");
    bytes[6] ^= 1;
    std::fs::write(&cache, bytes).expect("write stale cache");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--prelude-cache"])
        .arg(&cache)
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stale"));
    assert!(!source.with_extension("zm").exists());

    for path in [
        source.clone(),
        source.with_extension("zm"),
        source.with_extension("zman"),
        cache,
    ] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}

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
    manifest
        .validate_prelude_fingerprint(zun_prelude::fingerprint())
        .expect("CLI manifest must identify the compiled prelude");
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
fn build_aot_runs_a_boolean_entry_point() {
    let root = std::env::temp_dir().join(format!("zun-aot-bool-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): boolean { return true; }\n")
        .expect("write source");

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
    assert_eq!(String::from_utf8_lossy(&executable.stdout).trim(), "true");

    for extension in ["ts", "zm", "zman", "o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_preserves_unsigned_64_bit_entry_points() {
    let root = std::env::temp_dir().join(format!("zun-aot-u64-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(
        &source,
        "function main(): u64 { let value: u64 = 9223372036854775807; return value + value + 1; }\n",
    )
    .expect("write source");

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
    assert_eq!(
        String::from_utf8_lossy(&executable.stdout).trim(),
        "18446744073709551615"
    );

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
    std::fs::write(
        &source,
        "function main(): string { if (true) { return \"managed\"; } return \"fallback\"; }\n",
    )
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
