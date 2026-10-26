//! Release packaging recipe (PLAN §4.1). Dev tool, not shipped.
//!
//! `cargo run -p xtask -- package` runs the whole recipe and then the end-to-end check:
//!   1. `cargo build -p zun_runtime --release`
//!   2. locate `zun_runtime.dll` / `libzun_runtime.so` / `libzun_runtime.dylib`
//!   3. zstd-compress it to `target/package/zun-runtime.<ext>.zst`
//!   4. `cargo build -p zun_cli --release` with `ZUN_EMBED_RUNTIME=<blob>`
//!   5. print artifact paths and sizes
//!   6. verify: fresh `ZUN_HOME`, run the embedded `zun install`, assert the extracted
//!      runtime's SHA-256 equals the raw runtime's, and a second install is a no-op.
//!
//! `cargo run -p xtask -- verify` re-runs only step 6 against an existing build.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

type R<T> = Result<T, String>;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("target"))
}

fn ext() -> &'static str {
    if cfg!(windows) {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

fn runtime_artifact() -> PathBuf {
    let name = if cfg!(windows) { "zun_runtime.dll".to_string() } else { format!("libzun_runtime.{}", ext()) };
    target_dir().join("release").join(name)
}

fn cli_artifact() -> PathBuf {
    target_dir().join("release").join(if cfg!(windows) { "zun.exe" } else { "zun" })
}

fn blob_path() -> PathBuf {
    target_dir().join("package").join(format!("zun-runtime.{}.zst", ext()))
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn size(p: &Path) -> R<u64> {
    std::fs::metadata(p).map(|m| m.len()).map_err(|e| format!("{}: {e}", p.display()))
}

fn cargo(args: &[&str], embed: Option<&Path>) -> R<()> {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.args(args).current_dir(root());
    if let Some(p) = embed {
        c.env("ZUN_EMBED_RUNTIME", p);
    }
    let st = c.status().map_err(|e| format!("cargo: {e}"))?;
    st.success().then_some(()).ok_or_else(|| format!("`cargo {}` failed", args.join(" ")))
}

fn package() -> R<()> {
    cargo(&["build", "-p", "zun_runtime", "--release"], None)?;
    let rt = runtime_artifact();
    let raw = std::fs::read(&rt).map_err(|e| format!("{}: {e}", rt.display()))?;
    let blob = blob_path();
    std::fs::create_dir_all(blob.parent().unwrap()).map_err(|e| e.to_string())?;
    let z = zstd::stream::encode_all(&raw[..], 19).map_err(|e| format!("zstd: {e}"))?;
    std::fs::write(&blob, &z).map_err(|e| format!("{}: {e}", blob.display()))?;
    cargo(&["build", "-p", "zun_cli", "--release"], Some(&blob))?;
    let cli = cli_artifact();
    println!("runtime       {}  {} bytes", rt.display(), raw.len());
    println!("runtime.zst   {}  {} bytes", blob.display(), z.len());
    println!("embedded zun  {}  {} bytes", cli.display(), size(&cli)?);
    verify()
}

fn run_install(cli: &Path, home: &Path) -> R<String> {
    let out = Command::new(cli).arg("install").env("ZUN_HOME", home).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        return Err(format!("`zun install` failed: {}{}", text, String::from_utf8_lossy(&out.stderr)));
    }
    Ok(text)
}

fn verify() -> R<()> {
    let raw = std::fs::read(runtime_artifact()).map_err(|e| e.to_string())?;
    let want = sha_hex(&raw);
    let home = target_dir().join("package").join(format!("verify-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let cli = cli_artifact();
    let first = run_install(&cli, &home)?;
    let got_path = home.join("modules").join(format!("zun-runtime.{}", ext()));
    let got = std::fs::read(&got_path).map_err(|e| format!("{}: {e}", got_path.display()))?;
    let got_hex = sha_hex(&got);
    if got_hex != want {
        return Err(format!("hash mismatch: extracted {got_hex}, runtime {want}"));
    }
    if !first.contains("installed runtime") {
        return Err(format!("first install did not write: {first}"));
    }
    let second = run_install(&cli, &home)?;
    if !second.contains("up to date") {
        return Err(format!("second install was not a no-op: {second}"));
    }
    println!("verify: first install   -> {}", first.trim());
    println!("verify: second install  -> {}", second.trim());
    println!("verify: sha256 {got_hex} ({} bytes) matches runtime", got.len());
    let _ = std::fs::remove_dir_all(&home);
    println!("verify: OK");
    Ok(())
}

fn main() {
    let r = match std::env::args().nth(1).as_deref() {
        Some("package") => package(),
        Some("verify") => verify(),
        _ => Err("usage: cargo run -p xtask -- <package|verify>".into()),
    };
    if let Err(e) = r {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}
