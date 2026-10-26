//! Embedding seam for the runtime blob.
//!
//! If `ZUN_EMBED_RUNTIME` points at a zstd-compressed `zun_runtime` artifact it is
//! copied into `OUT_DIR`; otherwise an empty blob is emitted and the crate sees
//! `has_embedded_runtime == false`. Release packaging recipe (not part of this crate):
//! build `zun_runtime --release`, zstd-compress the artifact, set `ZUN_EMBED_RUNTIME`
//! to it, then build `zun_cli`.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=ZUN_EMBED_RUNTIME");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("zun_runtime.zst");
    match std::env::var("ZUN_EMBED_RUNTIME") {
        Ok(path) if !path.is_empty() => {
            println!("cargo:rerun-if-changed={path}");
            std::fs::copy(&path, &out)
                .unwrap_or_else(|e| panic!("ZUN_EMBED_RUNTIME `{path}`: {e}"));
        }
        _ => std::fs::write(&out, []).unwrap(),
    }
}
