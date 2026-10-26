//! Zun install layout (`~/.zun/`) and self-provisioning of the embedded runtime.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

static EMBEDDED_RUNTIME: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/zun_runtime.zst"));

/// True when this build carries a zstd-compressed runtime blob.
pub fn has_embedded_runtime() -> bool {
    !EMBEDDED_RUNTIME.is_empty()
}

/// Host shared-library extension.
pub fn runtime_ext() -> &'static str {
    if cfg!(windows) {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// Resolve the Zun home: `$ZUN_HOME`, else `<home>/.zun`.
pub fn zun_home() -> Result<PathBuf, String> {
    if let Some(home) = std::env::var_os("ZUN_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".zun"))
        .ok_or_else(|| format!("cannot resolve home directory (${var} unset); set ZUN_HOME"))
}

/// Create `bin/`, `cache/`, `modules/` under `home` (no-op if present).
pub fn ensure_layout(home: &Path) -> Result<(), String> {
    for sub in ["bin", "cache", "modules"] {
        let dir = home.join(sub);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    Ok(())
}

/// Outcome of a provisioning run.
#[derive(Debug, PartialEq, Eq)]
pub enum Provision {
    /// No runtime available in this build.
    NoEmbeddedRuntime,
    /// Target already matched the expected content hash.
    UpToDate(PathBuf),
    /// Target was (re)written.
    Written(PathBuf),
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Decompress `blob` (zstd) into `<home>/modules/zun-runtime.<ext>`; idempotent,
/// hash-verified, written via temp file + rename. Empty blob means "none embedded".
pub fn provision_blob(home: &Path, blob: &[u8]) -> Result<Provision, String> {
    ensure_layout(home)?;
    if blob.is_empty() {
        return Ok(Provision::NoEmbeddedRuntime);
    }
    let data = zstd::stream::decode_all(blob).map_err(|e| format!("corrupt runtime blob: {e}"))?;
    let want = sha(&data);
    let target = home.join("modules").join(format!("zun-runtime.{}", runtime_ext()));
    if let Ok(existing) = std::fs::read(&target) {
        if sha(&existing) == want {
            return Ok(Provision::UpToDate(target));
        }
    }
    let tmp = target.with_extension(format!("{}.tmp{}", runtime_ext(), std::process::id()));
    std::fs::write(&tmp, &data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", target.display()));
    }
    Ok(Provision::Written(target))
}

/// Provision from the blob embedded in this build.
pub fn provision(home: &Path) -> Result<Provision, String> {
    provision_blob(home, EMBEDDED_RUNTIME)
}

/// `zun install` entry point; prints status.
pub fn install() -> Result<(), String> {
    let home = zun_home()?;
    match provision(&home)? {
        Provision::NoEmbeddedRuntime => println!(
            "zun: layout ready at {}; no embedded runtime in this build; set ZUN_EMBED_RUNTIME at package time",
            home.display()
        ),
        Provision::UpToDate(p) => println!("zun: runtime up to date at {}", p.display()),
        Provision::Written(p) => println!("zun: installed runtime to {}", p.display()),
    }
    Ok(())
}

/// Installed binary file name for this platform.
pub fn bin_name() -> &'static str {
    if cfg!(windows) { "zun.exe" } else { "zun" }
}

/// Renamed-aside previous binary name for this platform.
pub fn outdated_name() -> &'static str {
    if cfg!(windows) { "zun-outdated.exe" } else { "zun-outdated" }
}

/// Sentinel file name marking that `outdated_name()` should be removed on next start.
pub const UPGRADED_SENTINEL: &str = "zun.upgraded";

/// Cheap startup check (two stats in the common case): if the upgrade sentinel
/// exists, delete the outdated binary and clear the sentinel. A locked outdated
/// binary leaves the sentinel in place so a later start retries.
pub fn cleanup_after_upgrade(home: &Path) {
    let bin = home.join("bin");
    let sentinel = bin.join(UPGRADED_SENTINEL);
    if !sentinel.exists() {
        return;
    }
    match std::fs::remove_file(bin.join(outdated_name())) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return, // still locked/mapped; retry next start
    }
    let _ = std::fs::remove_file(&sentinel);
}

/// Swap in a new `zun` binary from `from`: old -> `zun-outdated`, new copied
/// atomically (temp + rename), sentinel touched, then the runtime area is
/// re-provisioned from `blob`.
pub fn upgrade_with(home: &Path, from: &Path, blob: &[u8]) -> Result<Provision, String> {
    ensure_layout(home)?;
    if !from.is_file() {
        return Err(format!("{}: not a file", from.display()));
    }
    let bin = home.join("bin");
    let target = bin.join(bin_name());
    let outdated = bin.join(outdated_name());
    let tmp = bin.join(format!("{}.new{}", bin_name(), std::process::id()));
    std::fs::copy(from, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if target.exists() {
        let _ = std::fs::remove_file(&outdated); // replace any prior outdated
        if let Err(e) = std::fs::rename(&target, &outdated) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{}: {e}", outdated.display()));
        }
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", target.display()));
    }
    std::fs::write(bin.join(UPGRADED_SENTINEL), b"")
        .map_err(|e| format!("{}: {e}", UPGRADED_SENTINEL))?;
    provision_blob(home, blob)
}

/// `zun upgrade --from <path>` entry point.
/// Seam: Phase 7 replaces `--from` with a registry download of the new binary.
pub fn upgrade(from: &Path) -> Result<(), String> {
    let home = zun_home()?;
    let r = upgrade_with(&home, from, EMBEDDED_RUNTIME)?;
    println!("zun: upgraded {} ({r:?})", home.join("bin").join(bin_name()).display());
    Ok(())
}
