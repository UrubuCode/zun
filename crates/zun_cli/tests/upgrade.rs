use zun_cli::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zun-upgrade-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn upgrade_swaps_and_reprovisions() {
    let home = tmp("swap");
    ensure_layout(&home).unwrap();
    let bin = home.join("bin");
    std::fs::write(bin.join(bin_name()), b"old").unwrap();
    std::fs::write(bin.join(outdated_name()), b"older").unwrap();
    let new = home.join("new-zun");
    std::fs::write(&new, b"new").unwrap();
    let payload = b"rt payload".repeat(10);
    let blob = zstd::stream::encode_all(&payload[..], 3).unwrap();

    let r = upgrade_with(&home, &new, &blob).unwrap();
    assert!(matches!(r, Provision::Written(_)));
    assert_eq!(std::fs::read(bin.join(outdated_name())).unwrap(), b"old");
    assert_eq!(std::fs::read(bin.join(bin_name())).unwrap(), b"new");
    assert!(bin.join(UPGRADED_SENTINEL).exists());
    let rt = home.join("modules").join(format!("zun-runtime.{}", runtime_ext()));
    assert_eq!(std::fs::read(rt).unwrap(), payload);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn upgrade_without_prior_install_and_missing_source() {
    let home = tmp("fresh");
    assert!(upgrade_with(&home, &home.join("nope"), &[]).is_err());
    let new = home.join("new-zun");
    std::fs::write(&new, b"new").unwrap();
    upgrade_with(&home, &new, &[]).unwrap();
    assert_eq!(std::fs::read(home.join("bin").join(bin_name())).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn cleanup_removes_outdated_and_is_idempotent() {
    let home = tmp("clean");
    ensure_layout(&home).unwrap();
    let bin = home.join("bin");
    cleanup_after_upgrade(&home); // nothing present
    std::fs::write(bin.join(outdated_name()), b"old").unwrap();
    cleanup_after_upgrade(&home); // no sentinel: untouched
    assert!(bin.join(outdated_name()).exists());
    std::fs::write(bin.join(UPGRADED_SENTINEL), b"").unwrap();
    cleanup_after_upgrade(&home);
    assert!(!bin.join(outdated_name()).exists());
    assert!(!bin.join(UPGRADED_SENTINEL).exists());
    std::fs::write(bin.join(UPGRADED_SENTINEL), b"").unwrap();
    cleanup_after_upgrade(&home); // sentinel but no outdated
    assert!(!bin.join(UPGRADED_SENTINEL).exists());
    let _ = std::fs::remove_dir_all(&home);
}
