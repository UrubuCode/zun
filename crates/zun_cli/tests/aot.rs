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

    for path in [
        source.clone(),
        source.with_extension("zm"),
        source.with_extension("zman"),
        source.with_extension("o"),
    ] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir(root);
}
