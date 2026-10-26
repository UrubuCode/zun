use std::time::Duration;
use zun_cli::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zun-install-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn layout_created_and_idempotent() {
    let home = tmp("layout");
    ensure_layout(&home).unwrap();
    ensure_layout(&home).unwrap();
    for s in ["bin", "cache", "modules"] {
        assert!(home.join(s).is_dir());
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn zun_home_env_resolution() {
    let home = tmp("env");
    // Only test that does set the variable in this binary.
    std::env::set_var("ZUN_HOME", &home);
    assert_eq!(zun_home().unwrap(), home);
}

#[test]
fn provision_extract_idempotent_and_repair() {
    let home = tmp("prov");
    let payload = b"synthetic runtime payload".repeat(100);
    let blob = zstd::stream::encode_all(&payload[..], 3).unwrap();
    let target = home.join("modules").join(format!("zun-runtime.{}", runtime_ext()));

    assert_eq!(provision_blob(&home, &blob).unwrap(), Provision::Written(target.clone()));
    assert_eq!(std::fs::read(&target).unwrap(), payload);
    let m1 = std::fs::metadata(&target).unwrap().modified().unwrap();

    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(provision_blob(&home, &blob).unwrap(), Provision::UpToDate(target.clone()));
    assert_eq!(std::fs::metadata(&target).unwrap().modified().unwrap(), m1);

    std::fs::write(&target, b"corrupt").unwrap();
    assert_eq!(provision_blob(&home, &blob).unwrap(), Provision::Written(target.clone()));
    assert_eq!(std::fs::read(&target).unwrap(), payload);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn dev_build_without_embed_is_graceful() {
    let home = tmp("dev");
    assert_eq!(provision_blob(&home, &[]).unwrap(), Provision::NoEmbeddedRuntime);
    assert!(home.join("modules").is_dir());
    if !has_embedded_runtime() {
        assert_eq!(provision(&home).unwrap(), Provision::NoEmbeddedRuntime);
    }
    let _ = std::fs::remove_dir_all(&home);
}
