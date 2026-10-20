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
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(source.with_extension("zm").is_file());
    assert!(source.with_extension("zman").is_file());
    assert!(source.with_extension("o").is_file());
    assert!(source.with_extension("exe").is_file());
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
