#![allow(dead_code)]

use zun_codegen::RetVal;
use zun_interp::value::Value;
use zun_interp::CaptureOutput;

pub fn compile(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true).unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

/// Run the entry point through the interpreter, returning its value.
pub fn interp_entry(module: &zun_module::Module) -> Value {
    let mut out = CaptureOutput::default();
    zun_interp::run(module, &mut out).unwrap()
}

/// Assert the JIT and interpreter agree on the entry point's result.
pub fn assert_equivalent(src: &str) {
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();

    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(
        jit.is_compiled(entry),
        "entry should be JIT-eligible:\n{src}"
    );

    let jit_result = jit.call_entry(entry).expect("jit call");
    let interp_result = interp_entry(&module);

    let matches = match (&jit_result, &interp_result) {
        (RetVal::Float(a), Value::Float(b, _)) => a == b,
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        (RetVal::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    };
    assert!(
        matches,
        "JIT {jit_result:?} vs interpreter {interp_result:?} for:\n{src}"
    );
}

pub fn compile_oxc(src: &str) -> zun_module::Module {
    let ast = zun_syntax::oxc::parse_subset(src, true).expect("oxc parse");
    let mut diags = Vec::new();
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

/// Run `body` as `main(): int` through both frontends and expect `expected`.
pub fn assert_regex_program(body: &str, expected: i64) {
    let src = format!("function main(): int {{ {body} }}");
    for (name, module) in [("hand", compile(&src)), ("oxc", compile_oxc(&src))] {
        match interp_entry(&module) {
            Value::Int(v, _) => assert_eq!(v, expected, "{name}: {src}"),
            other => panic!("{name}: unexpected result {other:?} for {src}"),
        }
    }
}
