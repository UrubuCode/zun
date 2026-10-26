//! `zun run <file.zm>` picks the engine from the module's `codegen` capability.

use std::process::Command;

const EVAL: &str = r#"function main() { let f = new Function("a", "return a*2.0;"); print((f(21.0)) as number); }"#;
const PLAIN: &str = r#"function main() { print(6.0 * 7.0); }"#;

fn write_zm(name: &str, src: &str) -> (std::path::PathBuf, bool) {
    let module = zun_compiler::compile_with_default_std(src).expect("compile");
    let dir = std::env::temp_dir().join("zun-run-zm-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, module.write()).unwrap();
    (path, module.requires_codegen)
}

fn run(path: &std::path::Path) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_zun")).arg("run").arg(path).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn eval_program_selects_eval_capable_path() {
    let (path, codegen) = write_zm("eval.zm", EVAL);
    assert!(codegen, "compiler must flag the module as requiring codegen");
    let (ok, stdout, stderr) = run(&path);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "42");
}

#[test]
fn eval_program_on_compiler_free_path_is_disallowed() {
    let (path, _) = write_zm("eval_free.zm", EVAL);
    let bytes = std::fs::read(path).unwrap();
    let err = zun_vm::execute_aot(&bytes).expect_err("must be disallowed").to_string();
    assert!(err.contains("disallowed"), "{err}");
}

#[test]
fn plain_program_runs_on_compiler_free_path() {
    let (path, codegen) = write_zm("plain.zm", PLAIN);
    assert!(!codegen);
    let (ok, stdout, stderr) = run(&path);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "42");
    let bytes = std::fs::read(path).unwrap();
    assert!(zun_vm::execute_aot(&bytes).is_ok());
}
