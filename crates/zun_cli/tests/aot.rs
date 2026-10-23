use std::process::Command;

/// Locate the `zun_vm` static library the AOT fallback tests link against.
///
/// Five tests need it and each used to find it inline and `return` when it was
/// absent — which the harness reports as a pass. A whole group of AOT tests
/// could therefore be green having run nothing, and on a clean checkout that
/// is exactly what happened.
///
/// Worse is the other case. The library is a `staticlib` artifact that is not
/// in any test's dependency graph, so `cargo test` never rebuilds it. A stale
/// one links fine and then fails at runtime with an empty stderr, because the
/// interpreter inside it does not recognize opcodes the freshly compiled `.zm`
/// now contains. That cost real debugging time, so staleness is a hard error
/// with instructions rather than a mystery.
///
/// Returns `None` when the library is absent, after saying so. Set
/// `ZUN_REQUIRE_AOT_RUNTIME=1` to make that a failure instead, which is what
/// CI should do so the group cannot silently stop testing anything.
fn aot_runtime_library() -> Option<std::path::PathBuf> {
    let test_exe = std::env::current_exe().expect("test executable path");
    let name = if cfg!(windows) {
        "zun_vm.lib"
    } else {
        "libzun_vm.a"
    };
    let library = test_exe
        .parent()
        .and_then(std::path::Path::parent)
        .map(|path| path.join(name));

    let Some(library) = library.filter(|path| path.is_file()) else {
        let message = format!(
            "{name} is not built, so this AOT fallback test verified nothing. Build it with `cargo build -p zun_vm`."
        );
        if std::env::var_os("ZUN_REQUIRE_AOT_RUNTIME").is_some() {
            panic!("{message}");
        }
        eprintln!("SKIPPED: {message}");
        return None;
    };

    // Freshness is measured against the runtime's sources, not against
    // another build artifact: within a single `cargo build` the staticlib and
    // the binaries are written seconds apart in an order that means nothing.
    if let Some(newest) = newest_runtime_source() {
        let built = std::fs::metadata(&library).and_then(|meta| meta.modified());
        if let Ok(built) = built {
            assert!(
                built >= newest,
                "{name} is older than the runtime sources, so it holds an older runtime than the code under test. It links, then fails at run time with no diagnostic. Rebuild it with `cargo build -p zun_vm`."
            );
        }
    }

    Some(library)
}

/// Most recent modification time across every crate's `src`, which is what
/// the static library has to be at least as new as. Only `src` counts: edits
/// to tests say nothing about the runtime.
fn newest_runtime_source() -> Option<std::time::SystemTime> {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent()?;
    let mut newest: Option<std::time::SystemTime> = None;
    let mut pending = vec![crates.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                // Only descend into a crate root or its `src`.
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if directory == crates || name == "src" {
                    pending.push(path);
                }
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "rs") {
                if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                    newest = Some(newest.map_or(modified, |best| best.max(modified)));
                }
            }
        }
    }
    newest
}

#[test]
fn build_uses_native_oxc_for_javascript_inputs() {
    let root = std::env::temp_dir().join(format!("zun-oxc-cli-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.js");
    std::fs::write(&source, "export default function main() {}\n").expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(source.with_extension("zm").is_file());

    for path in [source.clone(), source.with_extension("zm"), source.with_extension("zman")] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_uses_native_oxc_for_mts_inputs() {
    let root = std::env::temp_dir().join(format!("zun-oxc-mts-cli-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.mts");
    std::fs::write(
        &source,
        "export default function main(): int { let value: int = 0; for (let index: int = 0; index < 1; index = index + 1) { value = value + 7; } return value; } export { main };\n",
    )
        .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(source.with_extension("zm").is_file());

    for path in [source.clone(), source.with_extension("zm"), source.with_extension("zman")] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_uses_native_oxc_for_commonjs_inputs() {
    let root = std::env::temp_dir().join(format!("zun-oxc-cjs-cli-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.cjs");
    std::fs::write(&source, "function main() {}\n").expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(source.with_extension("zm").is_file());
    for path in [source.clone(), source.with_extension("zm"), source.with_extension("zman")] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}

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
    assert_eq!(manifest.dependencies, vec!["zun:std"]);

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
    assert_eq!(manifest.metadata_object.as_deref(), Some("main.meta.o"));
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
        source.with_extension("meta.o"),
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

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe"] {
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

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_preserves_f32_entry_abi() {
    let root = std::env::temp_dir().join(format!("zun-aot-f32-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): f32 { return 7.5; }\n")
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
    assert_eq!(String::from_utf8_lossy(&executable.stdout).trim(), "7.5");

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_preserves_i32_entry_abi() {
    let root = std::env::temp_dir().join(format!("zun-aot-i32-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temp directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): int { return 42; }\n")
        .expect("write source");

    let build = std::process::Command::new(env!("CARGO_BIN_EXE_zun"))
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun build");
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));

    let executable = root.join("main.exe");
    let run = std::process::Command::new(&executable)
        .output()
        .expect("run AOT executable");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");

    for suffix in ["ts", "zm", "zman", "o", "meta.o", "exe", "aot.rs", "pdb"] {
        let path = root.join(format!("main.{suffix}"));
        let _ = std::fs::remove_file(path);
    }
    std::fs::remove_dir(&root).expect("remove temp directory");
}

#[test]
fn build_aot_preserves_u32_entry_abi() {
    let root = std::env::temp_dir().join(format!("zun-aot-u32-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): u32 { return 42; }\n")
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
    assert_eq!(String::from_utf8_lossy(&executable.stdout).trim(), "42");

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe", "aot.rs", "pdb"] {
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

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe"] {
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
        .env("ZUN_RUNTIME_LIB", root.join("missing-runtime.lib"))
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not AOT-eligible"));
    assert!(source.with_extension("zm").is_file());
    assert!(source.with_extension("zman").is_file());
    let manifest = zun_loader::ModuleManifest::read_file(source.with_extension("zman"))
        .expect("read fallback manifest");
    assert!(manifest.native_object.is_none());
    assert!(!source.with_extension("exe").exists());

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_rejects_a_stale_explicit_lld_override() {
    let root = std::env::temp_dir().join(format!("zun-aot-lld-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): int { return 7; }\n").expect("write source");
    let missing = root.join("missing-lld.exe");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .env("ZUN_LLD", &missing)
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing-lld.exe"));

    for extension in ["ts", "zm", "zman", "o", "meta.o", "exe"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_runs_scalar_interpreter_fallback_with_runtime_library() {
    let Some(runtime) = aot_runtime_library() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("zun-aot-runtime-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(
        &source,
        "function main(): int { let value: any = 23; return value as int; }\n",
    )
    .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .env("ZUN_RUNTIME_LIB", &runtime)
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let executable = source.with_extension("exe");
    assert!(executable.is_file(), "{}", String::from_utf8_lossy(&output.stderr));
    let manifest = zun_loader::ModuleManifest::read_file(source.with_extension("zman"))
        .expect("read fallback manifest");
    assert_eq!(manifest.native_object, None);
    assert_eq!(manifest.metadata_object.as_deref(), Some("main.meta.o"));
    assert!(manifest.runtime_library.is_some());
    let run = Command::new(&executable).output().expect("run fallback executable");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "23");

    for extension in ["ts", "zm", "zman", "meta.o", "exe", "aot.rs"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_runs_string_interpreter_fallback_with_runtime_library() {
    let Some(runtime) = aot_runtime_library() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("zun-aot-string-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): string { return \"managed\"; }\n")
        .expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .env("ZUN_RUNTIME_LIB", &runtime)
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let executable = source.with_extension("exe");
    assert!(executable.is_file(), "{}", String::from_utf8_lossy(&output.stderr));
    let run = Command::new(&executable).output().expect("run fallback executable");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "managed");
    for extension in ["ts", "zm", "zman", "meta.o", "exe", "aot.rs"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_runs_unsigned_interpreter_fallback_with_runtime_library() {
    let Some(runtime) = aot_runtime_library() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("zun-aot-u64-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): u64 { let value: any = 23; return value as u64; }\n")
        .expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .env("ZUN_RUNTIME_LIB", &runtime)
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let executable = source.with_extension("exe");
    assert!(executable.is_file(), "{}", String::from_utf8_lossy(&output.stderr));
    let run = Command::new(&executable).output().expect("run fallback executable");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "23");
    for extension in ["ts", "zm", "zman", "meta.o", "exe", "aot.rs"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_preserves_boolean_interpreter_fallback_output() {
    let Some(runtime) = aot_runtime_library() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("zun-aot-bool-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): boolean { let value: any = true; return value as boolean; }\n")
        .expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .env("ZUN_RUNTIME_LIB", &runtime)
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let run = Command::new(source.with_extension("exe")).output().expect("run fallback executable");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "true");
    for extension in ["ts", "zm", "zman", "meta.o", "exe", "aot.rs"] {
        let _ = std::fs::remove_file(source.with_extension(extension));
    }
    let _ = std::fs::remove_dir(root);
}

#[test]
fn build_aot_runs_floating_interpreter_fallback_with_runtime_library() {
    let Some(runtime) = aot_runtime_library() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("zun-aot-f64-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create temporary directory");
    let source = root.join("main.ts");
    std::fs::write(&source, "function main(): number { let value: any = 2.5; return value as number; }\n")
        .expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_zun"))
        .env("ZUN_RUNTIME_LIB", &runtime)
        .args(["build", "--aot"])
        .arg(&source)
        .output()
        .expect("run zun");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let run = Command::new(source.with_extension("exe")).output().expect("run fallback executable");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "2.5");
    for extension in ["ts", "zm", "zman", "meta.o", "exe", "aot.rs"] {
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
