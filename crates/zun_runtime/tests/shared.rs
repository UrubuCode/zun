//! Smoke + measurement tests for the shared runtime cdylib.

use std::path::PathBuf;

const HELLO: &str = r#"function main() { print("hello"); }"#;

fn hello_zm() -> Vec<u8> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(HELLO, &mut diags);
    assert!(diags.is_empty(), "{diags:?}");
    let hir = zun_sema::check(HELLO, &ast, &mut diags);
    assert!(diags.is_empty(), "{diags:?}");
    zun_ilgen::lower(&hir).write()
}

/// `target/<profile>/zun_runtime.<ext>` derived from the test executable path.
fn cdylib_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap();
    dir.join(format!(
        "{}zun_runtime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ))
}

#[test]
fn dynamic_load_and_run() {
    let path = cdylib_path();
    assert!(path.exists(), "missing {path:?}");
    let zm = hello_zm();
    unsafe {
        let lib = libloading::Library::new(&path).expect("load cdylib");
        let ver: libloading::Symbol<extern "C" fn() -> u32> = lib.get(b"zun_rt_abi_version").unwrap();
        assert_eq!(ver(), 2);
        let run: libloading::Symbol<unsafe extern "C" fn(*const u8, usize) -> i32> =
            lib.get(b"zun_rt_run").unwrap();
        assert_eq!(run(zm.as_ptr(), zm.len()), 0);
        assert_eq!(run(std::ptr::null(), 0), -1);
        let junk = [1u8, 2, 3];
        assert_eq!(run(junk.as_ptr(), junk.len()), -2);
    }
}

/// Run: `cargo build -p zun_runtime --release` then
/// `cargo test -p zun_runtime --release --test shared -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_sizes() {
    let path = cdylib_path();
    let dll = std::fs::read(&path).expect("release cdylib");
    let z = zstd::encode_all(&dll[..], 19).unwrap();
    let zm = hello_zm();
    println!("cdylib: {} bytes ({:.2} MB)", dll.len(), dll.len() as f64 / 1048576.0);
    println!("zstd-19: {} bytes ({:.2} MB)", z.len(), z.len() as f64 / 1048576.0);
    println!("ratio: {:.2}x ({:.1}% of original)", dll.len() as f64 / z.len() as f64, 100.0 * z.len() as f64 / dll.len() as f64);
    println!("hello .zm: {} bytes ({:.2} KB)", zm.len(), zm.len() as f64 / 1024.0);
}

const EVAL_SRC: &str = r#"function main() { let f = new Function("a", "return a*2.0;"); print((f(21.0)) as number); }"#;

fn eval_zm() -> Vec<u8> {
    zun_compiler::compile_with_default_std(EVAL_SRC).expect("compile").write()
}

#[test]
fn eval_entry_matches_build_flavour() {
    let zm = eval_zm();
    unsafe {
        let lib = libloading::Library::new(cdylib_path()).expect("load cdylib");
        let has: libloading::Symbol<extern "C" fn() -> u32> = lib.get(b"zun_rt_has_eval").unwrap();
        let run: libloading::Symbol<unsafe extern "C" fn(*const u8, usize) -> i32> =
            lib.get(b"zun_rt_run").unwrap();
        let run_eval: libloading::Symbol<unsafe extern "C" fn(*const u8, usize) -> i32> =
            lib.get(b"zun_rt_run_eval").unwrap();
        // The compiler-free entry always disallows code generation.
        assert_eq!(run(zm.as_ptr(), zm.len()), -2);
        if has() == 1 {
            assert_eq!(run_eval(zm.as_ptr(), zm.len()), 0);
        } else {
            assert_eq!(run_eval(zm.as_ptr(), zm.len()), -4);
        }
        let hello = hello_zm();
        assert_eq!(run(hello.as_ptr(), hello.len()), 0);
        if has() == 1 {
            assert_eq!(run_eval(hello.as_ptr(), hello.len()), 0);
        }
    }
}
