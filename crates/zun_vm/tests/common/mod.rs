#![allow(dead_code)]

use zun_interp::CaptureOutput;

/// Compile `src` with the std prelude, run it, and return the output lines.
pub fn run_with_std(user: &str) -> Vec<String> {
    run_source(&zun_prelude::with_prelude(user))
}

/// Compile `src` to a module, serialize it, and run the bytes, returning the
/// captured output lines.
pub fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true).unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);

    // Go through the on-disk format so the test also exercises serialization.
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

/// Compile `src` through the OXC front-end (standard TS: `const`, `for..of`,
/// computed-symbol methods, spread) and run it.
pub fn run_oxc(src: &str) -> Vec<String> {
    let module = zun_compiler::compile_with_oxc(src, true).expect("compiles");
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

/// Compile and run `src`, expecting a runtime failure; return the error text.
pub fn run_source_expect_error(src: &str) -> String {
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true).unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    match zun_vm::execute_with(&bytes, &mut out) {
        Ok(_) => panic!("program should have failed at runtime"),
        Err(error) => error.to_string(),
    }
}

/// Compile `user` together with the `Array` class source (so its Zun-extension
/// methods resolve) and run it.
pub fn run_with_array_class(user: &str) -> Vec<String> {
    let src = format!("{user}\n{}", zun_prelude::ARRAY);
    run_source(&src)
}

/// Build `a` as a fresh `[1.0, 2.0, 3.0]` array in a program preamble.
pub fn array_123() -> &'static str {
    "let a = new Array(3); a[0] = 1.0; a[1] = 2.0; a[2] = 3.0;"
}

/// Compile `src` (with std prelude) to a module without running it, so tests can
/// inspect the synthesized regex matcher methods.
pub fn compile_with_std(user: &str) -> zun_module::Module {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(&src, true).unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    zun_ilgen::lower(&hir)
}
