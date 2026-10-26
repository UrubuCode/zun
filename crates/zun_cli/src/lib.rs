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
